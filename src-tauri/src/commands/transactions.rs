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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostingSummary {
    pub account: String,
    pub amount: Option<String>,
    pub commodity: Option<String>,
    pub comment: Option<String>,
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
        .enumerate()
        .map(|(i, txn)| {
            TransactionSummary {
                index: i,
                date: txn.date.format("%Y-%m-%d").to_string(),
                status: format!("{:?}", txn.status),
                description: txn.description.clone(),
                comment: txn.comment.clone(),
                postings: txn
                    .postings
                    .iter()
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
            }
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
        .nth(index)
        .ok_or("Transaction not found")?;

    Ok(TransactionSummary {
        index,
        date: txn.date.format("%Y-%m-%d").to_string(),
        status: format!("{:?}", txn.status),
        description: txn.description.clone(),
        comment: txn.comment.clone(),
        postings: txn
            .postings
            .iter()
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
    })
}
