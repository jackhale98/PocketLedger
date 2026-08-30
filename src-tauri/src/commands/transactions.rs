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
    /// First amount only — kept for older callers. A multi-commodity posting
    /// (possible after cost conversion or balancing) lists everything in
    /// `amounts`, and `has_hidden_details` is set on the transaction.
    pub amount: Option<String>,
    pub commodity: Option<String>,
    /// Every amount on the posting, in commodity order. Empty for an elided
    /// amount that could not be inferred.
    pub amounts: Vec<AmountSummary>,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmountSummary {
    pub quantity: String,
    pub commodity: String,
}

fn amounts_of(p: &hledger_core::balance::ResolvedPosting) -> Vec<AmountSummary> {
    p.amount
        .amounts
        .iter()
        .map(|(commodity, qty)| AmountSummary {
            quantity: qty.to_string(),
            commodity: commodity.clone(),
        })
        .collect()
}

fn posting_summary(p: &hledger_core::balance::ResolvedPosting) -> PostingSummary {
    let amounts = amounts_of(p);
    PostingSummary {
        account: p.account.full.clone(),
        amount: amounts.first().map(|a| a.quantity.clone()),
        commodity: amounts.first().map(|a| a.commodity.clone()),
        amounts,
        comment: p.comment.clone(),
    }
}

/// Anything the edit form can't show, at transaction or posting level. The
/// posting-level check is shared with the edit path (`posting_has_extras`)
/// so the badge and the "restructure refused" rule never disagree.
fn has_hidden_details(
    txn: &hledger_parser::ast::Transaction,
    resolved: &hledger_core::balance::ResolvedTransaction,
) -> bool {
    txn.code.is_some()
        || txn.secondary_date.is_some()
        || !txn.tags.is_empty()
        || txn.postings.iter().any(super::journal::posting_has_extras)
        || resolved.postings.iter().any(|p| p.amount.amounts.len() > 1)
}

/// Summarize forecast-generated transactions. They have no journal entry
/// behind them, so `index` is a position in this list only and must never be
/// passed to update_transaction/delete_transaction.
pub fn summarize_generated(
    generated: &[hledger_core::balance::ResolvedTransaction],
    limit: usize,
) -> Vec<TransactionSummary> {
    generated
        .iter()
        .take(limit)
        .enumerate()
        .map(|(i, txn)| TransactionSummary {
            index: i,
            date: txn.date.format("%Y-%m-%d").to_string(),
            status: format!("{:?}", txn.status),
            description: txn.description.clone(),
            comment: txn.comment.clone(),
            postings: txn.postings.iter().map(posting_summary).collect(),
            has_hidden_details: false,
        })
        .collect()
}

fn summarize(
    loaded: &super::journal::LoadedJournal,
    txn: &hledger_core::balance::ResolvedTransaction,
    ast_index: usize,
) -> TransactionSummary {
    let hidden = loaded
        .nth_transaction(ast_index)
        .map(|(ast, _, _)| has_hidden_details(ast, txn))
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
            .map(posting_summary)
            .collect(),
        has_hidden_details: hidden,
    }
}

#[tauri::command]
pub async fn list_transactions(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<TransactionSummary>, String> {
    let app_state = crate::lock_or_recover(&state);
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

/// Search transactions with the hledger query language (acct:, desc:, amt:,
/// date:, cur:, status:, tag:, not:, plus bare account/description terms).
#[tauri::command]
pub async fn search_transactions(
    query: String,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<TransactionSummary>, String> {
    let app_state = crate::lock_or_recover(&state);
    let loaded = app_state
        .journal
        .as_ref()
        .ok_or("No journal loaded")?;

    let parsed = hledger_core::query::parse_query(&query)?;

    Ok(loaded
        .ledger
        .transactions()
        .filter(|txn| parsed.matches_transaction(txn))
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
    let app_state = crate::lock_or_recover(&state);
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

#[cfg(test)]
mod tests {
    use super::summarize;

    #[test]
    fn every_commodity_of_a_posting_is_reported() {
        let dir = std::env::temp_dir().join(format!("pockethledger-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main.journal");
        std::fs::write(
            &main,
            "2024-01-01 Mixed\n    assets:a  $1.00\n    assets:a  2 EUR\n    assets:b  $-1.00\n    assets:b  -2 EUR\n\n2024-01-02 Plain\n    a  $1.00\n    b\n",
        )
        .unwrap();
        let loaded = crate::commands::journal::load_journal(&main.to_string_lossy()).unwrap();
        let txn = loaded.ledger.transactions().next().unwrap();
        let s = summarize(&loaded, txn, 0);
        // Postings to the same account may be merged; either way every
        // commodity must be present somewhere.
        let all: Vec<String> = s
            .postings
            .iter()
            .flat_map(|p| p.amounts.iter().map(|a| a.commodity.clone()))
            .collect();
        assert!(all.contains(&"$".to_string()) && all.contains(&"EUR".to_string()), "{all:?}");
        let any_multi = s.postings.iter().any(|p| p.amounts.len() > 1);
        assert_eq!(s.has_hidden_details, any_multi);
        for p in &s.postings {
            assert_eq!(p.amount.as_deref(), p.amounts.first().map(|a| a.quantity.as_str()));
        }
        let plain = summarize(&loaded, loaded.ledger.transactions().nth(1).unwrap(), 1);
        assert!(!plain.has_hidden_details);
        assert_eq!(plain.postings[1].amounts.len(), 1, "elided amount is inferred");
    }
}
