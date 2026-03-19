mod commands;

use std::sync::Mutex;

pub struct AppState {
    pub journal: Option<commands::journal::LoadedJournal>,
}

impl Default for AppState {
    fn default() -> Self {
        Self { journal: None }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(Mutex::new(AppState::default()))
        .invoke_handler(tauri::generate_handler![
            commands::journal::open_journal,
            commands::journal::get_journal_info,
            commands::journal::save_journal,
            commands::journal::create_journal,
            commands::journal::add_transaction,
            commands::journal::update_transaction,
            commands::journal::delete_transaction,
            commands::journal::suggest_accounts,
            commands::journal::suggest_descriptions,
            commands::journal::suggest_payees,
            commands::transactions::list_transactions,
            commands::transactions::get_transaction,
            commands::reports::balance_report,
            commands::reports::register_report,
            commands::reports::balance_sheet_report,
            commands::reports::income_statement_report,
            commands::reports::cash_flow_report,
            commands::reports::net_worth_series,
            commands::reports::account_balance_series,
            commands::reports::income_expense_chart,
            commands::reports::expense_breakdown_chart,
            commands::reports::list_accounts_with_balances,
            commands::reports::list_commodities,
            commands::reconciliation::start_reconciliation,
            commands::reconciliation::toggle_reconciliation_posting,
            commands::reconciliation::get_reconciliation_state,
            commands::reconciliation::finish_reconciliation,
            commands::reconciliation::cancel_reconciliation,
            commands::budget::get_budgets,
            commands::budget::budget_vs_actual,
            commands::budget::budget_summary_chart,
            commands::budget::save_budget,
            commands::budget::list_budget_accounts,
            commands::journal::switch_journal,
            commands::csv_import::preview_csv_import,
            commands::csv_import::import_csv,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
