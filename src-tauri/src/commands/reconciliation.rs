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
    let app_state = crate::lock_or_recover(&state);
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
    let mut recon = crate::lock_or_recover(&RECONCILIATION);
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
    let app_state = crate::lock_or_recover(&state);

    let mut recon = crate::lock_or_recover(&RECONCILIATION);
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
    let recon = crate::lock_or_recover(&RECONCILIATION);
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
    add_assertion: Option<bool>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<super::journal::JournalSummary, String> {
    let mut app_state = crate::lock_or_recover(&state);

    // Take the session out (it is consumed either way on success).
    let active = {
        let mut recon = crate::lock_or_recover(&RECONCILIATION);
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
        let mut recon = crate::lock_or_recover(&RECONCILIATION);
        *recon = Some(active);
        return Err(format!(
            "Cleared balance differs from the statement by {}. Keep reconciling, or finish anyway to save the partial state.",
            difference
        ));
    }

    // A reconciled statement is worth pinning: a balance assertion on the
    // statement date makes hledger (and this app) catch any later drift —
    // the plain-text-accounting habit this flow exists to support.
    let assertion = if add_assertion.unwrap_or(false) && active.session.is_reconciled() {
        Some(assertion_transaction(&active.session, &app_state)?)
    } else {
        None
    };

    let changes = active.session.changes();
    if changes.is_empty() {
        return match assertion {
            Some(text) => super::journal::apply_append_to_main(&mut app_state, &text),
            None => super::journal::make_summary_result(&app_state),
        };
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

    if let Some(text) = assertion {
        summary = Some(super::journal::apply_append_to_main(&mut app_state, &text)?);
    }

    summary.ok_or_else(|| "Nothing to save".to_string())
}

/// `DATE * Reconciled ACCOUNT` with a single zero posting asserting the
/// statement balance, written in the journal's style for that commodity.
fn assertion_transaction(
    session: &ReconciliationSession,
    app_state: &crate::AppState,
) -> Result<String, String> {
    use hledger_parser::ast::{
        AccountName, BalanceAssertion, Comment, Posting, PostingAmount, Transaction,
    };

    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    let st = session.state();
    let date = chrono::NaiveDate::parse_from_str(&st.statement_date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid statement date: {}", e))?;
    let balance = rust_decimal::Decimal::from_str_exact(st.statement_balance.trim())
        .map_err(|e| format!("Invalid balance: {}", e))?;
    let commodity = st.statement_commodity.clone();
    let mut style = loaded
        .parse_context
        .style_for(&commodity)
        .unwrap_or_else(|| writer::default_style_for(&commodity));
    style.precision = style.precision.max(balance.scale() as u8);

    let span = SourceSpan { start: 0, end: 0, line: 0 };
    let txn = Transaction {
        span: span.clone(),
        date,
        secondary_date: None,
        status: Status::Cleared,
        code: None,
        description: format!("Reconciled {}", st.account),
        comment: Some(Comment { text: "statement balance".to_string() }),
        tags: vec![],
        postings: vec![Posting {
            span,
            status: Status::Unmarked,
            account: AccountName::new(&st.account),
            amount: Some(PostingAmount {
                quantity: rust_decimal::Decimal::ZERO,
                commodity: commodity.clone(),
                style: style.clone(),
                cost: None,
                multiplier: false,
            }),
            balance_assertion: Some(BalanceAssertion {
                strong: false,
                inclusive: false,
                quantity: balance,
                commodity,
                style,
            }),
            comment: None,
            tags: vec![],
            is_virtual: false,
            virtual_balanced: false,
            date: None,
            date2: None,
        }],
    };
    Ok(writer::write_transaction(&txn, &loaded.writer_config))
}

#[tauri::command]
pub async fn cancel_reconciliation() -> Result<(), String> {
    let mut recon = crate::lock_or_recover(&RECONCILIATION);
    *recon = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assertion_transaction_is_hledger_valid_and_in_journal_style() {
        let dir = std::env::temp_dir().join(format!("pockethledger-recon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main.journal");
        std::fs::write(
            &main,
            "commodity 1.000,00 EUR\n\n2024-01-05 * Salary\n    assets:bank   2.500,00 EUR\n    income:salary\n\n2024-01-10 Rent\n    expenses:rent   1.000,00 EUR\n    assets:bank\n",
        )
        .unwrap();
        let app_state = crate::AppState {
            journal: Some(super::super::journal::load_journal(&main.to_string_lossy()).unwrap()),
            backup_dir: None,
            generation: 0,
            infer_market_prices: false,
        };
        let loaded = app_state.journal.as_ref().unwrap();
        let txns: Vec<_> = loaded.ledger.transactions().cloned().collect();
        let mut session = ReconciliationSession::new(
            &txns,
            "assets:bank",
            chrono::NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            rust_decimal::Decimal::new(150000, 2),
            "EUR",
        );
        session.toggle_posting(1);
        assert!(session.is_reconciled(), "{:?}", session.state());

        let text = assertion_transaction(&session, &app_state).unwrap();
        assert!(text.contains("2024-01-31 * Reconciled assets:bank"), "{text}");
        // Written in the journal's comma-decimal, dot-grouped style.
        assert!(text.contains("= 1.500,00 EUR"), "{text}");

        // hledger must accept the assertion against this journal.
        let mut whole = std::fs::read_to_string(&main).unwrap();
        whole.push('\n');
        whole.push_str(&text);
        std::fs::write(&main, &whole).unwrap();
        if let Ok(out) = std::process::Command::new("hledger")
            .args(["-f", &main.to_string_lossy(), "check", "assertions"])
            .output()
        {
            assert!(out.status.success(), "hledger: {}", String::from_utf8_lossy(&out.stderr));
        }
        // And this engine reads it back without an assertion warning.
        let reloaded = super::super::journal::load_journal(&main.to_string_lossy()).unwrap();
        assert!(reloaded.all_warnings().is_empty(), "{:?}", reloaded.all_warnings());
    }
}
