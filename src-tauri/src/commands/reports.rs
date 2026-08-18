use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use hledger_core::reports;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportParams {
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub account_filter: Option<String>,
    pub target_commodity: Option<String>,
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
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::BalanceRow>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::balance_report(
        &txns,
        params.account_filter.as_deref(),
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn register_report(
    account: String,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::RegisterRow>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::register_report(
        &txns,
        &account,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn balance_sheet_report(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<reports::FinancialStatement, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::balance_sheet(
        &txns,
        loaded.ledger.classifier(),
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn income_statement_report(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<reports::FinancialStatement, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::income_statement(
        &txns,
        loaded.ledger.classifier(),
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn cash_flow_report(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<reports::FinancialStatement, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::cash_flow(
        &txns,
        loaded.ledger.classifier(),
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn net_worth_series(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::TimeSeriesPoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::net_worth_series(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn account_balance_series(
    account: String,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::TimeSeriesPoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::account_series(
        &txns,
        loaded.ledger.price_db(),
        &account,
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn income_expense_chart(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::IncomeExpensePoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::income_expense_series(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
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
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
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
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
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
    params: Option<ReportParams>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::BalanceRow>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();

    if let Some(params) = params {
        if let Some(target) = params.target_commodity.as_deref() {
            if !target.is_empty() {
                return Ok(reports::balance_report_valued(
                    &txns,
                    params.account_filter.as_deref(),
                    parse_date(&params.date_from),
                    parse_date(&params.date_to),
                    target,
                    loaded.ledger.price_db(),
                ));
            }
        }
    }

    Ok(reports::balance_report(&txns, None, None, None))
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
