//! Forecast commands: manage the journal's periodic rules (`~ …`) and project
//! them forward.
//!
//! The same `~` rules also define budget goals — hledger has no way to mark a
//! rule as one or the other — so these commands deliberately operate on the
//! same items `budget.rs` reads.

use std::sync::Mutex;

use serde::Deserialize;
use tauri::State;

use hledger_core::forecast;
use hledger_parser::ast::JournalItem;
use hledger_parser::writer;

use super::reports::{parse_date, resolve_target_commodity, ReportParams};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveForecastPosting {
    pub account: String,
    /// Empty means an elided amount, which hledger infers to balance the rule.
    pub amount: Option<String>,
    pub commodity: Option<String>,
}

#[tauri::command]
pub async fn get_forecast_rules(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<forecast::ForecastRule>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    Ok(forecast::extract_rules(&loaded.journal))
}

/// Find a periodic transaction by source line; returns its span and file index.
fn find_periodic_by_line(
    loaded: &super::journal::LoadedJournal,
    line: usize,
) -> Option<(hledger_parser::ast::SourceSpan, usize)> {
    for (idx, item) in loaded.journal.items.iter().enumerate() {
        if let JournalItem::PeriodicTransaction(pt) = item {
            if pt.span.line == line {
                return Some((pt.span.clone(), loaded.item_files[idx]));
            }
        }
    }
    None
}

#[tauri::command]
pub async fn save_forecast_rule(
    period: String,
    description: String,
    postings: Vec<SaveForecastPosting>,
    replace_line: Option<usize>,
    // Which journal file to append a new rule to. Ignored when replacing,
    // since an existing rule is rewritten wherever it already lives.
    file_index: Option<usize>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<super::journal::JournalSummary, String> {
    // hledger ends the period expression at the first double space, so an
    // inner run of spaces would silently truncate the rule on reload; collapse
    // runs before validating and writing.
    let period = period.split_whitespace().collect::<Vec<_>>().join(" ");

    // Reject period expressions we can't honor rather than writing a rule
    // that silently generates nothing.
    if hledger_core::period::parse_period_expression(&period).is_none() {
        return Err(format!(
            "Unsupported period expression '{period}'. Supported: daily / weekly / monthly / quarterly / yearly, 'every N <unit>s', 'every Nth day of month', 'every <weekday>', each optionally with 'from DATE' and 'to DATE'."
        ));
    }
    // A description sits after a double space; a ';' would start a comment and
    // a newline would end the rule.
    if description.contains(';') || description.contains('\n') {
        return Err("A description cannot contain ';' or line breaks.".to_string());
    }

    let mut parsed: Vec<(String, Option<rust_decimal::Decimal>, String)> = Vec::new();
    let mut elided = 0;
    for p in postings {
        let account = p.account.trim().to_string();
        if account.is_empty() {
            return Err("A posting is missing its account.".to_string());
        }
        let amount = match p.amount.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            Some(raw) => Some(
                rust_decimal::Decimal::from_str_exact(raw)
                    .map_err(|e| format!("Invalid amount '{raw}': {e}"))?,
            ),
            None => {
                elided += 1;
                None
            }
        };
        parsed.push((account, amount, p.commodity.unwrap_or_default()));
    }

    if parsed.is_empty() {
        return Err("A recurring transaction needs at least one posting.".to_string());
    }
    // hledger infers at most one missing amount; more than one is an error at
    // load time, which would make the whole journal unreadable.
    if elided > 1 {
        return Err(
            "Only one posting may be left without an amount — hledger can infer just one."
                .to_string(),
        );
    }

    let mut app_state = state.lock().map_err(|e| e.to_string())?;

    let (text, target) = {
        let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
        let text = writer::write_periodic_transaction_full(
            &period,
            &description,
            &parsed,
            &loaded.writer_config,
        );

        let target = match replace_line {
            Some(line) => {
                let (span, file_idx) = find_periodic_by_line(loaded, line)
                    .ok_or("The rule to replace was not found (journal changed?)")?;
                let file_text = &loaded.files[file_idx].text;
                let patched = writer::patch_journal(file_text, &[(span, text.clone())])?;
                Some((file_idx, patched))
            }
            None => None,
        };
        (text, target)
    };

    match target {
        Some((file_idx, patched)) => super::journal::apply_file_edit(&mut app_state, file_idx, patched),
        None => super::journal::apply_append_to_file(&mut app_state, file_index.unwrap_or(0), &text),
    }
}

#[tauri::command]
pub async fn delete_forecast_rule(
    line: usize,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<super::journal::JournalSummary, String> {
    let mut app_state = state.lock().map_err(|e| e.to_string())?;

    let (file_idx, patched) = {
        let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
        let (span, file_idx) = find_periodic_by_line(loaded, line)
            .ok_or("Rule not found (journal changed?)")?;
        let file_text = &loaded.files[file_idx].text;
        let patched = writer::delete_from_journal(file_text, &span)?;
        (file_idx, patched)
    };

    super::journal::apply_file_edit(&mut app_state, file_idx, patched)
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastProjection {
    pub points: Vec<forecast::ProjectionPoint>,
    pub shortfall: Option<forecast::ShortfallAlert>,
    /// Last date backed by a recorded transaction; everything later is a
    /// projection.
    pub last_actual: Option<String>,
    /// End of the projected window.
    pub horizon: Option<String>,
    pub commodity: String,
    /// True when the journal has no periodic rules to project from.
    pub no_rules: bool,
    /// Rules that generated nothing, with the reason. hledger refuses to load
    /// a journal with an unbalanced rule under --forecast; we forecast what we
    /// can and report the rest.
    pub rule_errors: Vec<String>,
    /// Days between the last recorded transaction and today. A large value
    /// means the projection starts from a balance that hasn't been updated
    /// in a while, which the UI should say out loud.
    pub days_since_last_actual: Option<i64>,
}

/// Project cash flow for `account` (default: all asset accounts) forward to
/// the horizon, and report the first date the balance goes negative.
#[tauri::command]
pub async fn forecast_projection(
    account: Option<String>,
    horizon: Option<String>,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<ForecastProjection, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    // No account chosen: project every asset/cash account by classification,
    // so journals whose asset tree isn't literally named "assets" still work.
    let account = account.as_deref().map(str::trim).filter(|a| !a.is_empty());
    let selector = match account {
        Some(prefix) => forecast::AccountSelector::Prefix(prefix),
        None => forecast::AccountSelector::Assets(loaded.ledger.classifier()),
    };

    let real: Vec<_> = loaded.ledger.transactions().cloned().collect();
    let last_actual = real.iter().map(|t| t.date).max();
    let today = chrono::Local::now().date_naive();
    let horizon_date = parse_date(&horizon);
    // Project forward from today, not from the end of the journal — see
    // forecast::projection_window.
    let (all, rule_errors) = match forecast::projection_window(&real, today, horizon_date) {
        Some((start, end)) => {
            let outcome = forecast::forecast_checked(&loaded.journal, start, end);
            let errors = outcome
                .errors
                .iter()
                .map(|(line, msg)| format!("line {line}: {msg}"))
                .collect();
            let mut all = real.clone();
            all.extend(outcome.transactions);
            all.sort_by_key(|t| t.date);
            (all, errors)
        }
        None => (real.clone(), Vec::new()),
    };

    let no_rules = !loaded
        .journal
        .items
        .iter()
        .any(|i| matches!(i, JournalItem::PeriodicTransaction(_)));

    // Without a window the chart would span the entire journal, squeezing the
    // part the user cares about into the last few pixels. Default to a few
    // months of recent actuals for context; an explicit filter still wins.
    // Anchored on today rather than the last transaction: for a stale journal
    // the months before the projection hold nothing, so the chart starts at
    // the projection instead of implying old activity is recent.
    let chart_from = parse_date(&params.date_from)
        .or_else(|| today.checked_sub_months(chrono::Months::new(CONTEXT_MONTHS)));

    let points = forecast::cash_flow_projection(
        &all,
        last_actual,
        &selector,
        &commodity,
        loaded.ledger.price_db(),
        chart_from,
        parse_date(&params.date_to),
    );

    // Only projected shortfalls are interesting; a past overdraft is history
    // the user already lived through.
    let shortfall = forecast::first_shortfall(
        &all,
        &selector,
        &commodity,
        loaded.ledger.price_db(),
        rust_decimal::Decimal::ZERO,
        last_actual,
    );

    let effective_horizon =
        forecast::projection_window(&real, today, horizon_date).map(|(_, end)| end);

    Ok(ForecastProjection {
        points,
        shortfall,
        last_actual: last_actual.map(|d| d.format("%Y-%m-%d").to_string()),
        horizon: effective_horizon.map(|d| d.format("%Y-%m-%d").to_string()),
        commodity,
        no_rules,
        rule_errors,
        days_since_last_actual: last_actual.map(|d| (today - d).num_days()),
    })
}

/// Months of recorded history shown before the projection starts, for context.
const CONTEXT_MONTHS: u32 = 3;

/// The generated transactions themselves, for an "upcoming" list.
#[tauri::command]
pub async fn upcoming_transactions(
    horizon: Option<String>,
    limit: Option<usize>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<super::transactions::TransactionSummary>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let real: Vec<_> = loaded.ledger.transactions().cloned().collect();
    let today = chrono::Local::now().date_naive();
    let Some((start, end)) = forecast::projection_window(&real, today, parse_date(&horizon))
    else {
        return Ok(vec![]);
    };

    let generated = forecast::forecast_transactions(&loaded.journal, start, end);
    Ok(super::transactions::summarize_generated(
        &generated,
        limit.unwrap_or(usize::MAX),
    ))
}
