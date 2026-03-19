use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use hledger_core::csv_import;
use hledger_core::ledger::Ledger;
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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvPreview {
    pub transactions: Vec<CsvPreviewTransaction>,
    pub warnings: Vec<String>,
    pub rows_processed: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportResultResponse {
    pub imported_count: usize,
    pub warnings: Vec<String>,
    pub summary: super::journal::JournalSummary,
}

fn load_and_convert(csv_path: &str, rules_path: &str) -> Result<csv_import::CsvImportResult, String> {
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

#[tauri::command]
pub async fn preview_csv_import(
    csv_path: String,
    rules_path: String,
) -> Result<CsvPreview, String> {
    let result = load_and_convert(&csv_path, &rules_path)?;

    let preview_txns: Vec<CsvPreviewTransaction> = result
        .transactions
        .iter()
        .map(|txn| {
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
            }
        })
        .collect();

    Ok(CsvPreview {
        transactions: preview_txns,
        warnings: result.warnings,
        rows_processed: result.rows_processed,
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
    let loaded = app_state.journal.as_mut().ok_or("No journal loaded")?;

    let mut imported = 0;
    for &idx in &selected_indices {
        if let Some(txn) = result.transactions.get(idx) {
            let txn_text = writer::write_transaction(txn, &loaded.writer_config);

            if !loaded.source_text.ends_with('\n') {
                loaded.source_text.push('\n');
            }
            loaded.source_text.push('\n');
            loaded.source_text.push_str(&txn_text);
            imported += 1;
        }
    }

    // Write and re-resolve
    std::fs::write(&loaded.source_path, &loaded.source_text).map_err(|e| e.to_string())?;
    loaded.journal = hledger_parser::parse(&loaded.source_text).map_err(|e| e.to_string())?;
    loaded.ledger = Ledger::from_journal(&loaded.journal).map_err(|e| e.to_string())?;

    Ok(CsvImportResultResponse {
        imported_count: imported,
        warnings: result.warnings,
        summary: super::journal::make_summary_pub(loaded),
    })
}
