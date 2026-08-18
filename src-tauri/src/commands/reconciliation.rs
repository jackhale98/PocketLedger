use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Deserialize;
use tauri::State;

use hledger_core::reconciliation::{ReconciliationSession, ReconciliationState};
use hledger_parser::ast::{SourceSpan, Status, Transaction};
use hledger_parser::writer;

struct ActiveReconciliation {
    session: ReconciliationSession,
    /// AppState.generation at session start. Any journal mutation invalidates
    /// the session's indices — patching with stale indices marked the wrong
    /// transactions.
    generation: u64,
}

static RECONCILIATION: Mutex<Option<ActiveReconciliation>> = Mutex::new(None);

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
    let balance = rust_decimal::Decimal::from_str_exact(params.statement_balance.trim())
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
    *recon = Some(ActiveReconciliation {
        session,
        generation: app_state.generation,
    });

    Ok(result)
}

#[tauri::command]
pub async fn toggle_reconciliation_posting(
    index: usize,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<ReconciliationState, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;

    let mut recon = RECONCILIATION.lock().map_err(|e| e.to_string())?;
    let active = recon.as_mut().ok_or("No reconciliation in progress")?;
    if active.generation != app_state.generation {
        *recon = None;
        return Err(
            "The journal changed while reconciling; the session was cancelled. Start again."
                .to_string(),
        );
    }

    active.session.toggle_posting(index);
    Ok(active.session.state())
}

#[tauri::command]
pub async fn get_reconciliation_state() -> Result<Option<ReconciliationState>, String> {
    let recon = RECONCILIATION.lock().map_err(|e| e.to_string())?;
    Ok(recon.as_ref().map(|a| a.session.state()))
}

/// Apply a posting-level status change to an AST transaction. When the
/// transaction carries a top-level status, it is pushed down onto every
/// posting first, so clearing one posting can't silently (un)clear the other
/// account's side.
fn apply_posting_status(txn: &mut Transaction, posting_idx: usize, new_status: Status) {
    if txn.status != Status::Unmarked {
        for p in txn.postings.iter_mut() {
            if p.status == Status::Unmarked {
                p.status = txn.status;
            }
        }
        txn.status = Status::Unmarked;
    }

    if let Some(p) = txn.postings.get_mut(posting_idx) {
        p.status = new_status;
    }

    // Tidy: if every posting ended up with the same non-unmarked status,
    // lift it back to the transaction level (idiomatic journal style).
    if !txn.postings.is_empty() {
        let first = txn.postings[0].status;
        if first != Status::Unmarked && txn.postings.iter().all(|p| p.status == first) {
            txn.status = first;
            for p in txn.postings.iter_mut() {
                p.status = Status::Unmarked;
            }
        }
    }
}

#[tauri::command]
pub async fn finish_reconciliation(
    force: Option<bool>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<super::journal::JournalSummary, String> {
    let mut app_state = state.lock().map_err(|e| e.to_string())?;

    // Take the session out (it is consumed either way on success).
    let active = {
        let mut recon = RECONCILIATION.lock().map_err(|e| e.to_string())?;
        recon.take().ok_or("No reconciliation in progress")?
    };

    if active.generation != app_state.generation {
        return Err(
            "The journal changed while reconciling; nothing was written. Start again."
                .to_string(),
        );
    }

    if !active.session.is_reconciled() && !force.unwrap_or(false) {
        // Put the session back so the user can continue.
        let difference = active.session.difference();
        let mut recon = RECONCILIATION.lock().map_err(|e| e.to_string())?;
        *recon = Some(active);
        return Err(format!(
            "Cleared balance differs from the statement by {}. Keep reconciling, or finish anyway to save the partial state.",
            difference
        ));
    }

    let changes = active.session.changes();
    if changes.is_empty() {
        return super::journal::make_summary_result(&app_state);
    }

    // Group changes by resolved-transaction index.
    let mut by_txn: BTreeMap<usize, Vec<(usize, Status)>> = BTreeMap::new();
    for (ti, pi, status) in changes {
        by_txn.entry(ti).or_default().push((pi, status));
    }

    // Build span patches per file, addressing transactions by their SPANS
    // (the old date+description matching corrupted journals containing two
    // same-day purchases from the same payee).
    let mut patches_per_file: BTreeMap<usize, Vec<(SourceSpan, String)>> = BTreeMap::new();
    {
        let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
        let resolved: Vec<_> = loaded.ledger.transactions().collect();

        for (resolved_idx, posting_changes) in by_txn {
            let resolved_txn = resolved
                .get(resolved_idx)
                .ok_or("Reconciliation refers to a transaction that no longer exists")?;
            let ast_index = resolved_txn
                .postings
                .first()
                .map(|p| p.transaction_index)
                .ok_or("Transaction has no postings")?;
            let (ast_txn, _item_idx, file_idx) = loaded
                .nth_transaction(ast_index)
                .ok_or("Transaction not found in journal")?;

            let mut modified = ast_txn.clone();
            for (pi, status) in posting_changes {
                if pi >= modified.postings.len() {
                    return Err(
                        "Reconciliation refers to a posting that no longer exists".to_string()
                    );
                }
                apply_posting_status(&mut modified, pi, status);
            }

            let new_text = writer::write_transaction(&modified, &loaded.writer_config);
            patches_per_file
                .entry(file_idx)
                .or_default()
                .push((ast_txn.span.clone(), new_text));
        }
    }

    // Apply file by file through the validated safe-write path.
    let mut summary = None;
    for (file_idx, patches) in patches_per_file {
        let patched = {
            let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
            writer::patch_journal(&loaded.files[file_idx].text, &patches)?
        };
        summary = Some(super::journal::apply_file_edit(
            &mut app_state,
            file_idx,
            patched,
        )?);
        // NOTE: apply_file_edit reloads state; spans for later files remain
        // valid because each file's spans are relative to that file only and
        // the reload keeps file ordering (main first, includes in order).
    }

    summary.ok_or_else(|| "Nothing to save".to_string())
}

#[tauri::command]
pub async fn cancel_reconciliation() -> Result<(), String> {
    let mut recon = RECONCILIATION.lock().map_err(|e| e.to_string())?;
    *recon = None;
    Ok(())
}
