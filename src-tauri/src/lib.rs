mod commands;

use std::path::PathBuf;
use std::sync::Mutex;

pub struct AppState {
    pub journal: Option<commands::journal::LoadedJournal>,
    /// Where pre-edit backups go. Deliberately outside the journal's own
    /// folder: writing `<name>.bak` next to a journal kept in Dropbox or
    /// OneDrive doubles the sync traffic and the conflict surface on every
    /// save. None until setup resolves it, in which case backups are skipped.
    pub backup_dir: Option<PathBuf>,
    /// Bumped on every journal mutation or (re)load. Long-running flows like
    /// reconciliation hold indices into the resolved transaction list; a
    /// generation mismatch means those indices are stale and must not be
    /// used to patch the file.
    pub generation: u64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            journal: None,
            backup_dir: None,
            generation: 0,
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(Mutex::new(AppState::default()))
        .setup(|app| {
            use tauri::Manager;
            let dir = app.path().app_data_dir().ok().map(|d| d.join("backups"));
            if let Some(dir) = &dir {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(mut state) = app.state::<Mutex<AppState>>().lock() {
                state.backup_dir = dir;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::journal::open_journal,
            commands::journal::get_journal_info,
            commands::journal::journal_changed_on_disk,
            commands::journal::save_journal,
            commands::journal::create_journal,
            commands::journal::add_transaction,
            commands::journal::list_journal_files,
            commands::journal::update_transaction,
            commands::journal::delete_transaction,
            commands::journal::suggest_accounts,
            commands::journal::suggest_descriptions,
            commands::journal::suggest_payees,
            commands::journal::accounts_for_description,
            commands::transactions::list_transactions,
            commands::transactions::search_transactions,
            commands::transactions::get_transaction,
            commands::reports::periodic_balance,
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
            commands::reports::commodity_prices,
            commands::reports::journal_statistics,
            commands::reconciliation::start_reconciliation,
            commands::reconciliation::toggle_reconciliation_posting,
            commands::reconciliation::get_reconciliation_state,
            commands::reconciliation::finish_reconciliation,
            commands::reconciliation::cancel_reconciliation,
            commands::budget::budget_vs_actual,
            commands::budget::budget_summary_chart,
            commands::forecast::get_forecast_rules,
            commands::forecast::save_forecast_rule,
            commands::forecast::delete_forecast_rule,
            commands::forecast::forecast_projection,
            commands::forecast::upcoming_transactions,
            commands::reports::valuation_info,
            commands::journal::switch_journal,
            commands::csv_import::preview_csv_import,
            commands::csv_import::import_csv,
            commands::storage::platform_info,
            commands::storage::resolve_journal_ref,
            commands::storage::list_stored_journals,
            commands::storage::import_journal_file,
            commands::storage::delete_stored_journal,
            commands::storage::create_stored_journal,
            commands::storage::stash_picked_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
