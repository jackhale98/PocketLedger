use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use hledger_core::csv_import;
use hledger_parser::ast::JournalItem;
use hledger_parser::csv_rules;
use hledger_parser::writer;

use super::journal::normalize_path;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvPreviewTransaction {
    pub date: String,
    pub description: String,
    pub account1: String,
    pub account2: String,
    pub amount: String,
    pub commodity: String,
    pub comment: Option<String>,
    /// True when a transaction with the same date, amount and description
    /// already exists in the journal — re-importing an overlapping statement
    /// used to silently double every transaction.
    pub is_duplicate: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvPreview {
    pub transactions: Vec<CsvPreviewTransaction>,
    pub warnings: Vec<String>,
    pub rows_processed: usize,
    pub duplicate_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportResultResponse {
    pub imported_count: usize,
    pub skipped_duplicates: usize,
    pub warnings: Vec<String>,
    pub summary: super::journal::JournalSummary,
}

fn load_and_convert(
    csv_path: &str,
    rules_path: &str,
) -> Result<csv_import::CsvImportResult, String> {
    let rules_file = normalize_path(rules_path);
    let csv_file = normalize_path(csv_path);

    let rules_text = std::fs::read_to_string(&rules_file)
        .map_err(|e| format!("Cannot read rules file {}: {}", rules_file.display(), e))?;
    let rules = csv_rules::parse_csv_rules(&rules_text)
        .map_err(|e| format!("Rules parse error: {}", e))?;

    let csv_text = std::fs::read_to_string(&csv_file)
        .map_err(|e| format!("Cannot read CSV file {}: {}", csv_file.display(), e))?;

    csv_import::convert_csv(&csv_text, &rules)
}

fn existing_transactions(
    loaded: &super::journal::LoadedJournal,
) -> Vec<hledger_parser::ast::Transaction> {
    loaded
        .journal
        .items
        .iter()
        .filter_map(|item| match item {
            JournalItem::Transaction(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

#[tauri::command]
pub async fn preview_csv_import(
    csv_path: String,
    rules_path: String,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<CsvPreview, String> {
    let result = load_and_convert(&csv_path, &rules_path)?;

    let duplicates = {
        let app_state = state.lock().map_err(|e| e.to_string())?;
        match app_state.journal.as_ref() {
            Some(loaded) => csv_import::mark_probable_duplicates(
                &result.transactions,
                &existing_transactions(loaded),
            ),
            None => vec![false; result.transactions.len()],
        }
    };

    let preview_txns: Vec<CsvPreviewTransaction> = result
        .transactions
        .iter()
        .zip(duplicates.iter())
        .map(|(txn, is_dup)| {
            let p1 = &txn.postings[0];
            let p2 = txn.postings.get(1);
            let (amount, commodity) = p1
                .amount
                .as_ref()
                .map(|a| (a.quantity.to_string(), a.commodity.clone()))
                .unwrap_or_default();

            CsvPreviewTransaction {
                date: txn.date.format("%Y-%m-%d").to_string(),
                description: txn.description.clone(),
                account1: p1.account.full.clone(),
                account2: p2.map(|p| p.account.full.clone()).unwrap_or_default(),
                amount,
                commodity,
                comment: txn.comment.as_ref().map(|c| c.text.clone()),
                is_duplicate: *is_dup,
            }
        })
        .collect();

    let duplicate_count = preview_txns.iter().filter(|t| t.is_duplicate).count();

    Ok(CsvPreview {
        transactions: preview_txns,
        warnings: result.warnings,
        rows_processed: result.rows_processed,
        duplicate_count,
    })
}

#[tauri::command]
pub async fn import_csv(
    csv_path: String,
    rules_path: String,
    selected_indices: Vec<usize>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<CsvImportResultResponse, String> {
    let result = load_and_convert(&csv_path, &rules_path)?;

    let mut app_state = state.lock().map_err(|e| e.to_string())?;

    // Recompute duplicates at import time: the preview may be stale.
    let (duplicates, config) = {
        let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
        (
            csv_import::mark_probable_duplicates(
                &result.transactions,
                &existing_transactions(loaded),
            ),
            loaded.writer_config.clone(),
        )
    };

    // The user's selection is explicit; still, count how many selected rows
    // were flagged duplicates so the UI can report it.
    let mut addition = String::new();
    let mut imported = 0;
    let mut selected_duplicates = 0;
    for &idx in &selected_indices {
        if let Some(txn) = result.transactions.get(idx) {
            if duplicates.get(idx).copied().unwrap_or(false) {
                selected_duplicates += 1;
            }
            if !addition.is_empty() {
                addition.push('\n');
            }
            addition.push_str(&writer::write_transaction(txn, &config));
            imported += 1;
        }
    }

    if imported == 0 {
        return Err("No rows selected for import".to_string());
    }

    let summary = super::journal::apply_append_to_main(&mut app_state, &addition)?;

    let mut warnings = result.warnings;
    if selected_duplicates > 0 {
        warnings.push(format!(
            "{} imported row(s) look like duplicates of transactions already in the journal",
            selected_duplicates
        ));
    }

    Ok(CsvImportResultResponse {
        imported_count: imported,
        skipped_duplicates: 0,
        warnings,
        summary,
    })
}
