use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use hledger_core::budget;
use hledger_parser::ast::JournalItem;
use hledger_parser::writer;

use super::reports::{parse_date, resolve_target_commodity, ReportParams};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetInfo {
    /// The full period expression, e.g. "monthly" or "every 2 weeks from 2026-01".
    pub period: String,
    pub description: String,
    /// Source line of the periodic transaction — pass back to save_budget's
    /// replaceLine to edit in place, or to delete_budget.
    pub line: usize,
    pub entries: Vec<BudgetEntryInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetEntryInfo {
    pub account: String,
    pub amount: String,
    pub commodity: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveBudgetEntry {
    pub account: String,
    pub amount: String,
    pub commodity: String,
}

#[tauri::command]
pub async fn get_budgets(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<BudgetInfo>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let budgets = budget::extract_budgets(&loaded.journal);
    Ok(budgets
        .into_iter()
        .map(|b| BudgetInfo {
            period: b.period.raw.clone(),
            description: b.description,
            line: b.line,
            entries: b
                .entries
                .into_iter()
                .map(|e| BudgetEntryInfo {
                    account: e.account,
                    amount: e.amount.to_string(),
                    commodity: e.commodity,
                })
                .collect(),
        })
        .collect())
}

#[tauri::command]
pub async fn budget_vs_actual(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<budget::BudgetRow>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let budgets = budget::extract_budgets(&loaded.journal);
    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(budget::budget_vs_actual(
        &txns,
        &budgets,
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn budget_summary_chart(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<budget::BudgetSummaryPoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let budgets = budget::extract_budgets(&loaded.journal);
    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    // The date filter the UI shows now actually applies to the chart.
    Ok(budget::budget_summary_series(
        &txns,
        &budgets,
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

/// Find a periodic transaction item by its source line; returns (item span,
/// file index).
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
pub async fn save_budget(
    entries: Vec<SaveBudgetEntry>,
    period: String,
    replace_line: Option<usize>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<super::journal::JournalSummary, String> {
    // Reject period expressions the budget engine can't honor — writing them
    // would silently produce wrong goals.
    if budget::parse_period_expression(&period).is_none() {
        return Err(format!(
            "Unsupported period expression '{}'. Supported: daily / weekly / monthly / quarterly / yearly, 'every N <unit>s', optionally with 'from DATE' and 'to DATE'.",
            period
        ));
    }

    let postings: Vec<(String, rust_decimal::Decimal, String)> = entries
        .into_iter()
        .map(|e| {
            let qty = rust_decimal::Decimal::from_str_exact(e.amount.trim())
                .map_err(|err| format!("Invalid amount '{}': {}", e.amount, err))?;
            if e.account.trim().is_empty() {
                return Err("Budget entry with empty account".to_string());
            }
            Ok((e.account.trim().to_string(), qty, e.commodity))
        })
        .collect::<Result<Vec<_>, String>>()?;

    if postings.is_empty() {
        return Err("A budget needs at least one entry".to_string());
    }

    let mut app_state = state.lock().map_err(|e| e.to_string())?;

    let (text, target) = {
        let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

        let mut text = writer::write_periodic_transaction(&period, &postings, &loaded.writer_config);
        // hledger requires periodic transactions to balance; without this,
        // `hledger bal --budget` errors on the user's own journal. An elided
        // balancing posting keeps it valid.
        text.push_str(&" ".repeat(loaded.writer_config.indent));
        text.push_str("assets\n");

        let target = match replace_line {
            Some(line) => {
                let (span, file_idx) = find_periodic_by_line(loaded, line)
                    .ok_or("Budget to replace was not found (journal changed?)")?;
                let file_text = &loaded.files[file_idx].text;
                let patched = writer::patch_journal(file_text, &[(span, text.clone())])?;
                Some((file_idx, patched))
            }
            None => None,
        };
        (text, target)
    };

    match target {
        Some((file_idx, patched)) => {
            super::journal::apply_file_edit(&mut app_state, file_idx, patched)
        }
        None => super::journal::apply_append_to_main(&mut app_state, &text),
    }
}

#[tauri::command]
pub async fn delete_budget(
    line: usize,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<super::journal::JournalSummary, String> {
    let mut app_state = state.lock().map_err(|e| e.to_string())?;

    let (file_idx, patched) = {
        let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
        let (span, file_idx) = find_periodic_by_line(loaded, line)
            .ok_or("Budget not found (journal changed?)")?;
        let file_text = &loaded.files[file_idx].text;
        let patched = writer::delete_from_journal(file_text, &span)?;
        (file_idx, patched)
    };

    super::journal::apply_file_edit(&mut app_state, file_idx, patched)
}

#[tauri::command]
pub async fn list_budget_accounts(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<String>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let budgets = budget::extract_budgets(&loaded.journal);
    Ok(budget::budget_accounts(&budgets))
}
