use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use hledger_core::reports;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportParams {
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub account_filter: Option<String>,
    pub target_commodity: Option<String>,
    /// hledger query string filtering postings (acct:, desc:, amt:, ...).
    #[serde(default)]
    pub query: Option<String>,
    /// Extend reports with transactions materialized from periodic rules.
    #[serde(default)]
    pub forecast: Option<bool>,
}

/// Parse the params' query string, erroring loudly on bad syntax.
pub fn parse_query_param(params: &ReportParams) -> Result<Option<hledger_core::query::Query>, String> {
    match params.query.as_deref() {
        Some(q) if !q.trim().is_empty() => {
            let parsed = hledger_core::query::parse_query(q)?;
            Ok(Some(parsed))
        }
        _ => Ok(None),
    }
}

/// The transaction set a report should run over: real transactions, plus
/// forecast transactions up to date_to (or six months past the last real
/// transaction) when requested.
/// The transaction list every report should work from: the journal's own
/// transactions, optionally extended with the forecast, then narrowed by the
/// query. Reports that build this list by hand silently ignore the filter the
/// user set, which is the inconsistency this exists to prevent.
pub fn transactions_for(
    loaded: &super::journal::LoadedJournal,
    params: &ReportParams,
) -> Result<Vec<hledger_core::balance::ResolvedTransaction>, String> {
    let real: Vec<_> = loaded.ledger.transactions().cloned().collect();
    let txns = if params.forecast.unwrap_or(false) {
        hledger_core::forecast::with_forecast(&loaded.journal, &real, parse_date(&params.date_to))
    } else {
        real
    };
    match parse_query_param(params)? {
        Some(q) => Ok(hledger_core::query::retain_matching_postings(&txns, &q)),
        None => Ok(txns),
    }
}

/// Turn a requested chart interval into a bucket size. None leaves the report
/// to pick one from the range.
fn parse_step(interval: &Option<String>) -> Result<Option<reports::SeriesStep>, String> {
    match interval.as_deref().filter(|s| !s.is_empty() && *s != "auto") {
        None => Ok(None),
        Some(name) => reports::parse_series_step(name)
            .map(Some)
            .ok_or_else(|| format!("unknown interval '{name}'")),
    }
}

pub fn parse_date(s: &Option<String>) -> Option<chrono::NaiveDate> {
    s.as_ref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
}

/// Resolve the valuation target: the requested commodity if it actually
/// exists in the journal, else the journal's most-used commodity. The old
/// hardcoded "$" default sent every non-$ journal down a garbage path.
pub fn resolve_target_commodity(
    loaded: &super::journal::LoadedJournal,
    requested: Option<&str>,
) -> String {
    if let Some(req) = requested {
        if !req.is_empty() {
            let exists = loaded.ledger.transactions().any(|t| {
                t.postings
                    .iter()
                    .any(|p| p.amount.amounts.contains_key(req))
            });
            if exists {
                return req.to_string();
            }
        }
    }
    loaded.ledger.primary_commodity().unwrap_or_default()
}

#[tauri::command]
pub async fn balance_report(
    // "units" | "cost" | "market" | "gain"; omit for market value.
    valuation: Option<String>,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::BalanceRow>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let txns = transactions_for(loaded, &params)?;
    // Valued like every other report, so the currency the user picked applies
    // here too rather than only in the charts.
    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let mode = match valuation.as_deref().filter(|v| !v.is_empty()) {
        None => reports::ValuationMode::Market,
        Some(name) => reports::parse_valuation_mode(name)
            .ok_or_else(|| format!("unknown valuation mode '{name}'"))?,
    };
    let mut rows = reports::balance_report_mode(
        &txns,
        params.account_filter.as_deref(),
        parse_date(&params.date_from),
        parse_date(&params.date_to),
        &commodity,
        loaded.ledger.price_db(),
        mode,
    );
    hledger_core::styles::apply::balance_rows(&mut rows, loaded.ledger.styles());
    Ok(rows)
}

#[tauri::command]
pub async fn register_report(
    account: String,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::RegisterRow>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let txns = transactions_for(loaded, &params)?;
    let mut rows = reports::register_report(
        &txns,
        &account,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    );
    hledger_core::styles::apply::register_rows(&mut rows, loaded.ledger.styles());
    Ok(rows)
}

#[tauri::command]
pub async fn periodic_balance(
    interval: String,
    mode: Option<String>,
    depth: Option<usize>,
    // Narrow to a group of account types: "income-expense" for a period view
    // of what came in and went out, "assets-liabilities" for what is held.
    account_types: Option<String>,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<hledger_core::periodic_report::PeriodicBalanceReport, String> {
    use hledger_core::periodic_report::{AccumulationMode, ReportInterval};

    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let interval = match interval.as_str() {
        "weekly" | "W" => ReportInterval::Weekly,
        "monthly" | "M" => ReportInterval::Monthly,
        "quarterly" | "Q" => ReportInterval::Quarterly,
        "yearly" | "Y" => ReportInterval::Yearly,
        other => return Err(format!("unknown interval '{}'", other)),
    };
    let mode = match mode.as_deref() {
        None | Some("periodic") => AccumulationMode::Periodic,
        Some("cumulative") => AccumulationMode::Cumulative,
        Some("historical") => AccumulationMode::Historical,
        Some(other) => return Err(format!("unknown accumulation mode '{}'", other)),
    };

    // The query is still passed through: transactions_for has already applied
    // it, but the report also reads `depth:` off it.
    let query = parse_query_param(&params)?;
    let txns = transactions_for(loaded, &params)?;
    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());

    // Narrow before building the report, not after: its column totals are
    // computed from what it is given, so dropping rows later would leave
    // totals describing accounts no longer shown.
    use hledger_core::classify::AccountType;
    let txns = match account_types.as_deref() {
        None | Some("") | Some("all") => txns,
        Some("income-expense") => hledger_core::classify::retain_postings_of_types(
            &txns,
            loaded.ledger.classifier(),
            &[AccountType::Revenue, AccountType::Expense],
        ),
        Some("assets-liabilities") => hledger_core::classify::retain_postings_of_types(
            &txns,
            loaded.ledger.classifier(),
            &[AccountType::Asset, AccountType::Cash, AccountType::Liability],
        ),
        Some(other) => return Err(format!("unknown account group '{other}'")),
    };

    let mut report = hledger_core::periodic_report::periodic_balance_report(
        &txns,
        interval,
        mode,
        depth,
        query.as_ref(),
        parse_date(&params.date_from),
        parse_date(&params.date_to),
        &commodity,
        loaded.ledger.price_db(),
    );
    hledger_core::styles::apply::periodic(&mut report, loaded.ledger.styles());
    Ok(report)
}

#[tauri::command]
pub async fn balance_sheet_report(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<reports::FinancialStatement, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    let mut statement = reports::balance_sheet(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    );
    hledger_core::styles::apply::statement(&mut statement, loaded.ledger.styles());
    Ok(statement)
}

#[tauri::command]
pub async fn income_statement_report(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<reports::FinancialStatement, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    let mut statement = reports::income_statement(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    );
    hledger_core::styles::apply::statement(&mut statement, loaded.ledger.styles());
    Ok(statement)
}

#[tauri::command]
pub async fn cash_flow_report(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<reports::FinancialStatement, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    let mut statement = reports::cash_flow(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    );
    hledger_core::styles::apply::statement(&mut statement, loaded.ledger.styles());
    Ok(statement)
}

#[tauri::command]
pub async fn net_worth_series(
    interval: Option<String>,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::TimeSeriesPoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    let step = parse_step(&interval)?;
    Ok(reports::net_worth_series(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
        step,
    ))
}

#[tauri::command]
pub async fn account_balance_series(
    account: String,
    interval: Option<String>,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::TimeSeriesPoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    let step = parse_step(&interval)?;
    Ok(reports::account_series(
        &txns,
        loaded.ledger.price_db(),
        &account,
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
        step,
    ))
}

#[tauri::command]
pub async fn income_expense_chart(
    interval: Option<String>,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::IncomeExpensePoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    let step = parse_step(&interval)?;
    Ok(reports::income_expense_series(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
        step,
    ))
}

#[tauri::command]
pub async fn expense_breakdown_chart(
    params: ReportParams,
    parent_prefix: Option<String>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::PieSlice>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    Ok(reports::expense_breakdown(
        &txns,
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
        parent_prefix.as_deref(),
    ))
}

/// Info the UI needs to label valued charts honestly: which commodity the
/// numbers are in, and which commodities could NOT be valued (and are
/// therefore excluded from single-number charts).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValuationInfo {
    pub target_commodity: String,
    pub unconvertible: Vec<String>,
}

#[tauri::command]
pub async fn valuation_info(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<ValuationInfo, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    let date = parse_date(&params.date_to)
        .or_else(|| txns.last().map(|t| t.date))
        .unwrap_or_else(|| chrono::Local::now().date_naive());

    let unconvertible =
        reports::unconvertible_commodities(&txns, &commodity, loaded.ledger.price_db(), date);

    Ok(ValuationInfo {
        target_commodity: commodity,
        unconvertible,
    })
}

#[tauri::command]
pub async fn list_accounts_with_balances(
    valuation: Option<String>,
    params: Option<ReportParams>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::BalanceRow>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let effective = params.clone().unwrap_or_default();
    let txns = transactions_for(loaded, &effective)?;

    // No target currency means show what is actually held; with one, the
    // requested mode applies (market value unless asked otherwise).
    let target = params
        .as_ref()
        .and_then(|p| p.target_commodity.as_deref())
        .filter(|t| !t.is_empty());
    let mode = match valuation.as_deref().filter(|v| !v.is_empty()) {
        None => {
            if target.is_some() {
                reports::ValuationMode::Market
            } else {
                reports::ValuationMode::Units
            }
        }
        Some(name) => reports::parse_valuation_mode(name)
            .ok_or_else(|| format!("unknown valuation mode '{name}'"))?,
    };
    let mut rows = reports::balance_report_mode(
        &txns,
        params.as_ref().and_then(|p| p.account_filter.as_deref()),
        params.as_ref().and_then(|p| parse_date(&p.date_from)),
        params.as_ref().and_then(|p| parse_date(&p.date_to)),
        target.unwrap_or_default(),
        loaded.ledger.price_db(),
        mode,
    );
    hledger_core::styles::apply::balance_rows(&mut rows, loaded.ledger.styles());
    Ok(rows)
}

/// Price history per commodity pair, for a chart of what each is worth over
/// time.
#[tauri::command]
pub async fn commodity_prices(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<hledger_core::price_db::PriceSeries>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    Ok(loaded.ledger.price_db().series())
}

#[tauri::command]
pub async fn journal_statistics(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<reports::JournalStatistics, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    let txns = transactions_for(loaded, &params)?;
    Ok(reports::journal_statistics(&txns))
}

#[tauri::command]
pub async fn list_commodities(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<String>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let mut commodities = std::collections::BTreeSet::new();
    for txn in loaded.ledger.transactions() {
        for posting in &txn.postings {
            for commodity in posting.amount.amounts.keys() {
                if !commodity.is_empty() {
                    commodities.insert(commodity.clone());
                }
            }
        }
    }
    Ok(commodities.into_iter().collect())
}

/// Money-weighted (IRR) and time-weighted (TWR) return for a set of
/// investment accounts, matching `hledger roi`.
///
/// The investment query names the accounts holding the asset; the PnL query
/// names where its growth is booked. Anything moving between an investment
/// account and the outside world is a cash flow; anything moving in from PnL
/// is return, not a contribution.
#[tauri::command]
pub async fn roi_report(
    investment: String,
    pnl: Option<String>,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<hledger_core::roi::RoiReport, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    if investment.trim().is_empty() {
        return Err("Choose at least one investment account".into());
    }
    let investment_q = hledger_core::query::parse_query(&investment)?;
    let pnl_q = match pnl.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => Some(hledger_core::query::parse_query(p)?),
        None => None,
    };

    let txns = transactions_for(loaded, &params)?;
    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());

    let from = parse_date(&params.date_from)
        .or_else(|| txns.first().map(|t| t.date))
        .ok_or("Journal has no transactions")?;
    let to = parse_date(&params.date_to)
        .unwrap_or_else(|| chrono::Local::now().date_naive());
    if to < from {
        return Err("End date is before the start date".into());
    }

    Ok(hledger_core::roi::roi(
        &txns,
        &investment_q,
        pnl_q.as_ref(),
        from,
        to,
        &commodity,
        loaded.ledger.price_db(),
    ))
}
