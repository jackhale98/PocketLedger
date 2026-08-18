use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionSummary {
    pub index: usize,
    pub date: String,
    pub status: String,
    pub description: String,
    pub comment: Option<String>,
    pub postings: Vec<PostingSummary>,
    /// True when the transaction carries structure the edit form doesn't
    /// show (costs, assertions, tags, virtual postings, codes, secondary
    /// dates). Edits preserve it as long as the posting structure is kept.
    pub has_hidden_details: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostingSummary {
    pub account: String,
    pub amount: Option<String>,
    pub commodity: Option<String>,
    pub comment: Option<String>,
}

fn has_hidden_details(txn: &hledger_parser::ast::Transaction) -> bool {
    txn.code.is_some()
        || txn.secondary_date.is_some()
        || !txn.tags.is_empty()
        || txn.postings.iter().any(|p| {
            p.balance_assertion.is_some()
                || p.amount.as_ref().map_or(false, |a| a.cost.is_some())
                || p.is_virtual
                || !p.tags.is_empty()
                || p.status != hledger_parser::ast::Status::Unmarked
        })
}

fn summarize(
    loaded: &super::journal::LoadedJournal,
    txn: &hledger_core::balance::ResolvedTransaction,
    ast_index: usize,
) -> TransactionSummary {
    let hidden = loaded
        .nth_transaction(ast_index)
        .map(|(ast, _, _)| has_hidden_details(ast))
        .unwrap_or(false);

    TransactionSummary {
        index: ast_index,
        date: txn.date.format("%Y-%m-%d").to_string(),
        status: format!("{:?}", txn.status),
        description: txn.description.clone(),
        comment: txn.comment.clone(),
        postings: txn
            .postings
            .iter()
            // Generated (auto-rule) postings are not in the file; editing a
            // transaction "with" them would write them as real postings.
            .filter(|p| !p.generated)
            .map(|p| {
                let first_entry = p.amount.amounts.iter().next();
                PostingSummary {
                    account: p.account.full.clone(),
                    amount: first_entry.map(|(_, qty)| qty.to_string()),
                    commodity: first_entry.map(|(comm, _)| comm.clone()),
                    comment: p.comment.clone(),
                }
            })
            .collect(),
        has_hidden_details: hidden,
    }
}

#[tauri::command]
pub async fn list_transactions(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<TransactionSummary>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state
        .journal
        .as_ref()
        .ok_or("No journal loaded")?;

    Ok(loaded
        .ledger
        .transactions()
        .map(|txn| {
            let ast_index = txn
                .postings
                .first()
                .map(|p| p.transaction_index)
                .unwrap_or(0);
            summarize(loaded, txn, ast_index)
        })
        .collect())
}

#[tauri::command]
pub async fn get_transaction(
    index: usize,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<TransactionSummary, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state
        .journal
        .as_ref()
        .ok_or("No journal loaded")?;

    let txn = loaded
        .ledger
        .transactions()
        .find(|t| {
            t.postings
                .first()
                .map(|p| p.transaction_index)
                .unwrap_or(usize::MAX)
                == index
        })
        .ok_or("Transaction not found")?;

    Ok(summarize(loaded, txn, index))
}
