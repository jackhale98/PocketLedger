use std::sync::Mutex;

use serde::Deserialize;
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
    Ok(reports::balance_sheet(&txns, parse_date(&params.date_from), parse_date(&params.date_to)))
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

    let commodity = params.target_commodity.as_deref().unwrap_or("$");
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::net_worth_series(&txns, commodity, parse_date(&params.date_from), parse_date(&params.date_to)))
}

#[tauri::command]
pub async fn account_balance_series(
    account: String,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::TimeSeriesPoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = params.target_commodity.as_deref().unwrap_or("$");
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::account_series(&txns, &account, commodity, parse_date(&params.date_from), parse_date(&params.date_to)))
}

#[tauri::command]
pub async fn income_expense_chart(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::IncomeExpensePoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = params.target_commodity.as_deref().unwrap_or("$");
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::income_expense_series(&txns, commodity, parse_date(&params.date_from), parse_date(&params.date_to)))
}

#[tauri::command]
pub async fn expense_breakdown_chart(
    params: ReportParams,
    parent_prefix: Option<String>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::PieSlice>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = params.target_commodity.as_deref().unwrap_or("$");
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(reports::expense_breakdown(
        &txns,
        commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
        parent_prefix.as_deref(),
    ))
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

    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    let mut commodities = std::collections::BTreeSet::new();
    for txn in &txns {
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
