use std::sync::Mutex;

use serde::Deserialize;
use tauri::State;

use hledger_core::reconciliation::{ReconciliationSession, ReconciliationState};

/// Stored reconciliation session state (one at a time).
static RECONCILIATION: std::sync::Mutex<Option<ReconciliationSession>> =
    std::sync::Mutex::new(None);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartReconciliationParams {
    pub account: String,
    pub statement_date: String,
    pub statement_balance: String,
    pub commodity: String,
}

#[tauri::command]
pub async fn start_reconciliation(
    params: StartReconciliationParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<ReconciliationState, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let date = chrono::NaiveDate::parse_from_str(&params.statement_date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid date: {}", e))?;
    let balance = rust_decimal::Decimal::from_str_exact(&params.statement_balance)
        .map_err(|e| format!("Invalid balance: {}", e))?;

    let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
    let session = ReconciliationSession::new(
        &txns,
        &params.account,
        date,
        balance,
        &params.commodity,
    );

    let result = session.state();
    let mut recon = RECONCILIATION.lock().map_err(|e| e.to_string())?;
    *recon = Some(session);

    Ok(result)
}

#[tauri::command]
pub async fn toggle_reconciliation_posting(
    index: usize,
) -> Result<ReconciliationState, String> {
    let mut recon = RECONCILIATION.lock().map_err(|e| e.to_string())?;
    let session = recon.as_mut().ok_or("No reconciliation in progress")?;

    session.toggle_posting(index);
    Ok(session.state())
}

#[tauri::command]
pub async fn get_reconciliation_state() -> Result<Option<ReconciliationState>, String> {
    let recon = RECONCILIATION.lock().map_err(|e| e.to_string())?;
    Ok(recon.as_ref().map(|s| s.state()))
}

#[tauri::command]
pub async fn finish_reconciliation(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<super::journal::JournalSummary, String> {
    let changes = {
        let recon = RECONCILIATION.lock().map_err(|e| e.to_string())?;
        let session = recon.as_ref().ok_or("No reconciliation in progress")?;
        session.changes()
    };

    // Apply status changes to the journal source text
    let mut app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_mut().ok_or("No journal loaded")?;

    // For each change, update the transaction status in the source
    // We need to re-parse after each change since spans shift
    // Simple approach: modify the AST and rewrite affected transactions
    if !changes.is_empty() {
        // Get the transactions from the AST
        let mut txn_items: Vec<(usize, hledger_parser::ast::Transaction)> = loaded
            .journal
            .items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| match item {
                hledger_parser::ast::JournalItem::Transaction(t) => Some((idx, t.clone())),
                _ => None,
            })
            .collect();

        // Apply status changes
        for (txn_idx, _posting_idx, new_status) in &changes {
            if let Some((_, txn)) = txn_items.iter_mut().find(|(_, t)| {
                // Match by transaction index in the resolved list
                // The resolved list is sorted by date, but AST order may differ
                // Use the span line as identifier
                true // We'll use a simpler approach below
            }) {
                // For now, mark the whole transaction as cleared if any posting is cleared
                txn.status = *new_status;
            }
        }

        // Simpler approach: rebuild source text by re-serializing changed transactions
        let mut patches: Vec<(hledger_parser::ast::SourceSpan, String)> = Vec::new();

        for (txn_idx, _posting_idx, new_status) in &changes {
            // Find the AST transaction by matching against resolved transactions
            let resolved_txns: Vec<_> = loaded.ledger.transactions().collect();
            if *txn_idx >= resolved_txns.len() {
                continue;
            }
            let resolved = &resolved_txns[*txn_idx];

            // Find matching AST transaction by date + description
            for item in &mut loaded.journal.items {
                if let hledger_parser::ast::JournalItem::Transaction(ref mut t) = item {
                    if t.date == resolved.date && t.description == resolved.description {
                        t.status = *new_status;
                        let new_text = hledger_parser::writer::write_transaction(
                            t,
                            &loaded.writer_config,
                        );
                        patches.push((t.span.clone(), new_text));
                        break;
                    }
                }
            }
        }

        if !patches.is_empty() {
            loaded.source_text =
                hledger_parser::writer::patch_journal(&loaded.source_text, &patches);
            std::fs::write(&loaded.source_path, &loaded.source_text)
                .map_err(|e| e.to_string())?;
            loaded.journal = hledger_parser::parse(&loaded.source_text)
                .map_err(|e| e.to_string())?;
            loaded.ledger = hledger_core::ledger::Ledger::from_journal(&loaded.journal)
                .map_err(|e| e.to_string())?;
        }
    }

    // Clear the reconciliation session
    let mut recon = RECONCILIATION.lock().map_err(|e| e.to_string())?;
    *recon = None;

    Ok(super::journal::make_summary_pub(loaded))
}

#[tauri::command]
pub async fn cancel_reconciliation() -> Result<(), String> {
    let mut recon = RECONCILIATION.lock().map_err(|e| e.to_string())?;
    *recon = None;
    Ok(())
}
