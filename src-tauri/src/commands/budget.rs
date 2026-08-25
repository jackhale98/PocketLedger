use std::sync::Mutex;

use tauri::State;

use hledger_core::budget;

use super::reports::{parse_date, resolve_target_commodity, ReportParams};

#[tauri::command]
pub async fn budget_vs_actual(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<budget::BudgetComparison, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let budgets = budget::extract_budgets(&loaded.journal);
    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(budget::budget_comparison(
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
