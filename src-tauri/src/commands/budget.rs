use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use hledger_core::budget;
use hledger_parser::writer;

use super::reports::{parse_date, ReportParams};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetInfo {
    pub period: budget::BudgetPeriod,
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
            period: b.period,
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
    let commodity = params.target_commodity.as_deref().unwrap_or("$");
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(budget::budget_vs_actual(
        &txns,
        &budgets,
        commodity,
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
    let commodity = params.target_commodity.as_deref().unwrap_or("$");
    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    Ok(budget::budget_summary_series(&txns, &budgets, commodity))
}

#[tauri::command]
pub async fn save_budget(
    entries: Vec<SaveBudgetEntry>,
    period: String,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<super::journal::JournalSummary, String> {
    let postings: Vec<(String, rust_decimal::Decimal, String)> = entries
        .into_iter()
        .map(|e| {
            let qty = rust_decimal::Decimal::from_str_exact(&e.amount)
                .map_err(|err| format!("Invalid amount '{}': {}", e.amount, err))?;
            Ok((e.account, qty, e.commodity))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_mut().ok_or("No journal loaded")?;

    let text = writer::write_periodic_transaction(&period, &postings, &loaded.writer_config);

    // Append to source text
    if !loaded.source_text.ends_with('\n') {
        loaded.source_text.push('\n');
    }
    loaded.source_text.push('\n');
    loaded.source_text.push_str(&text);

    // Write to disk
    std::fs::write(&loaded.source_path, &loaded.source_text).map_err(|e| e.to_string())?;

    // Re-parse and re-resolve
    loaded.journal =
        hledger_parser::parse(&loaded.source_text).map_err(|e| e.to_string())?;
    loaded.ledger =
        hledger_core::ledger::Ledger::from_journal(&loaded.journal).map_err(|e| e.to_string())?;

    Ok(super::journal::make_summary_pub(loaded))
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
