use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use hledger_core::ledger::Ledger;
use hledger_parser::ast::{
    AccountName, AmountStyle, Comment, Journal, JournalItem, Posting, PostingAmount, Side,
    SourceSpan, Status, Tag, Transaction,
};
use hledger_parser::writer::{self, WriterConfig};

/// Normalize a path that might be a file:// URI (iOS returns these from dialogs)
/// into a regular filesystem PathBuf.
pub(crate) fn normalize_path(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("file://") {
        // Fast path: just strip the scheme and decode percent-encoding
        if let Ok(decoded) = urlencoding::decode(stripped) {
            return PathBuf::from(decoded.into_owned());
        }
        return PathBuf::from(stripped);
    }
    // Also handle url crate for edge cases
    if path.starts_with("file:") {
        if let Ok(url) = url::Url::parse(path) {
            if let Ok(p) = url.to_file_path() {
                return p;
            }
        }
    }
    PathBuf::from(path)
}

pub struct LoadedJournal {
    pub source_path: PathBuf,
    pub source_text: String,
    pub journal: Journal,
    pub ledger: Ledger,
    pub writer_config: WriterConfig,
    pub include_warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalSummary {
    pub file_name: String,
    pub transaction_count: usize,
    pub account_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewTransaction {
    pub date: String,
    pub status: String,
    pub description: String,
    pub comment: Option<String>,
    pub postings: Vec<NewPosting>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPosting {
    pub account: String,
    pub amount: Option<String>,
    pub commodity: Option<String>,
    pub comment: Option<String>,
}

pub fn make_summary_pub(loaded: &LoadedJournal) -> JournalSummary {
    make_summary(loaded)
}

fn make_summary(loaded: &LoadedJournal) -> JournalSummary {
    JournalSummary {
        file_name: loaded
            .source_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        transaction_count: loaded.ledger.transaction_count(),
        account_count: loaded.ledger.account_count(),
        warnings: loaded.include_warnings.clone(),
    }
}

fn load_journal(path: &str) -> Result<LoadedJournal, String> {
    let file_path = normalize_path(path);
    let source_text = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("Cannot read {}: {}", file_path.display(), e))?;
    let writer_config = writer::infer_config(&source_text);
    let mut journal = hledger_parser::parse(&source_text).map_err(|e| e.to_string())?;

    // Resolve include directives
    let base_dir = file_path.parent().map(|p| p.to_path_buf());
    let mut warnings = Vec::new();
    resolve_includes(&mut journal, base_dir.as_deref(), &mut warnings);

    let ledger = Ledger::from_journal(&journal).map_err(|e| e.to_string())?;

    Ok(LoadedJournal {
        source_path: file_path,
        source_text,
        journal,
        ledger,
        writer_config,
        include_warnings: warnings,
    })
}

fn resolve_includes(journal: &mut Journal, base_dir: Option<&std::path::Path>, warnings: &mut Vec<String>) {
    let mut new_items = Vec::new();
    for item in journal.items.drain(..) {
        match &item {
            JournalItem::IncludeDirective(inc) => {
                let inc_str = inc.path.trim();

                // Handle glob patterns (e.g. "include *.journal")
                if inc_str.contains('*') || inc_str.contains('?') {
                    let pattern = if let Some(base) = base_dir {
                        base.join(inc_str).to_string_lossy().to_string()
                    } else {
                        inc_str.to_string()
                    };
                    match glob::glob(&pattern) {
                        Ok(paths) => {
                            for entry in paths.flatten() {
                                match std::fs::read_to_string(&entry) {
                                    Ok(text) => {
                                        if let Ok(mut sub) = hledger_parser::parse(&text) {
                                            resolve_includes(&mut sub, entry.parent(), warnings);
                                            new_items.extend(sub.items);
                                        }
                                    }
                                    Err(e) => {
                                        warnings.push(format!("Could not include '{}': {}", entry.display(), e));
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warnings.push(format!("Invalid include pattern '{}': {}", inc_str, e));
                        }
                    }
                } else {
                    // Simple file path
                    let inc_path = if let Some(base) = base_dir {
                        base.join(inc_str)
                    } else {
                        PathBuf::from(inc_str)
                    };
                    match std::fs::read_to_string(&inc_path) {
                        Ok(text) => {
                            if let Ok(mut sub) = hledger_parser::parse(&text) {
                                resolve_includes(&mut sub, inc_path.parent(), warnings);
                                new_items.extend(sub.items);
                            }
                        }
                        Err(e) => {
                            warnings.push(format!("Could not include '{}': {} (resolved to {})", inc_str, e, inc_path.display()));
                        }
                    }
                }
                new_items.push(item); // Keep the directive for reference
            }
            _ => new_items.push(item),
        }
    }
    journal.items = new_items;
}

#[tauri::command]
pub async fn open_journal(
    path: String,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let loaded = load_journal(&path)?;
    let summary = make_summary(&loaded);

    let mut app_state = state.lock().map_err(|e| e.to_string())?;
    app_state.journal = Some(loaded);

    Ok(summary)
}

#[tauri::command]
pub async fn get_journal_info(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    Ok(make_summary(loaded))
}

#[tauri::command]
pub async fn save_journal(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<(), String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    std::fs::write(&loaded.source_path, &loaded.source_text).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn add_transaction(
    txn: NewTransaction,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let mut app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_mut().ok_or("No journal loaded")?;

    // Parse the new transaction into AST
    let date = chrono::NaiveDate::parse_from_str(&txn.date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid date: {}", e))?;

    let status = match txn.status.as_str() {
        "Cleared" | "cleared" | "*" => Status::Cleared,
        "Pending" | "pending" | "!" => Status::Pending,
        _ => Status::Unmarked,
    };

    let mut postings = Vec::new();
    for p in &txn.postings {
        let amount = if let Some(amt_str) = &p.amount {
            let quantity = rust_decimal::Decimal::from_str_exact(amt_str)
                .map_err(|e| format!("Invalid amount '{}': {}", amt_str, e))?;

            let commodity = p.commodity.clone().unwrap_or_default();
            let is_sym = commodity.len() == 1
                && "$€£¥₹₽₿₩₫₴₸₺₦₭"
                    .contains(commodity.chars().next().unwrap_or('x'));

            Some(PostingAmount {
                quantity,
                commodity: commodity.clone(),
                style: if is_sym {
                    AmountStyle {
                        commodity_side: Side::Left,
                        commodity_spaced: false,
                        decimal_mark: '.',
                        precision: 2,
                    }
                } else if commodity.is_empty() {
                    AmountStyle::default()
                } else {
                    AmountStyle {
                        commodity_side: Side::Right,
                        commodity_spaced: true,
                        decimal_mark: '.',
                        precision: 2,
                    }
                },
                cost: None,
            })
        } else {
            None
        };

        postings.push(Posting {
            span: SourceSpan { start: 0, end: 0, line: 0 },
            status: Status::Unmarked,
            account: AccountName::new(&p.account),
            amount,
            balance_assertion: None,
            comment: p.comment.as_ref().filter(|c| !c.is_empty()).map(|c| Comment {
                text: c.clone(),
            }),
            tags: vec![],
            is_virtual: false,
            virtual_balanced: false,
        });
    }

    let ast_txn = Transaction {
        span: SourceSpan { start: 0, end: 0, line: 0 },
        date,
        secondary_date: None,
        status,
        code: None,
        description: txn.description,
        comment: txn.comment.as_ref().filter(|c| !c.is_empty()).map(|c| Comment {
            text: c.clone(),
        }),
        tags: vec![],
        postings,
    };

    // Serialize to text
    let txn_text = writer::write_transaction(&ast_txn, &loaded.writer_config);

    // Append to source text (with blank line separator)
    if !loaded.source_text.ends_with('\n') {
        loaded.source_text.push('\n');
    }
    loaded.source_text.push('\n');
    loaded.source_text.push_str(&txn_text);

    // Write to file
    std::fs::write(&loaded.source_path, &loaded.source_text).map_err(|e| e.to_string())?;

    // Re-parse and re-resolve
    loaded.journal =
        hledger_parser::parse(&loaded.source_text).map_err(|e| e.to_string())?;
    loaded.ledger =
        Ledger::from_journal(&loaded.journal).map_err(|e| e.to_string())?;

    Ok(make_summary(loaded))
}

#[tauri::command]
pub async fn create_journal(
    path: String,
    default_currency: Option<String>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let currency = default_currency.unwrap_or_else(|| "$".to_string());
    let file_path = normalize_path(&path);

    let initial_content = format!(
        "; hledger journal\n\
         ; Created by PocketHLedger\n\
         \n\
         commodity {currency}1,000.00\n\
         \n\
         account assets\n\
         account assets:bank:checking\n\
         account assets:bank:savings\n\
         account assets:cash\n\
         account expenses\n\
         account expenses:food\n\
         account expenses:housing\n\
         account expenses:transport\n\
         account expenses:utilities\n\
         account income\n\
         account income:salary\n\
         account liabilities\n\
         account liabilities:credit card\n\
         account equity\n\
         account equity:opening balances\n\
         \n",
        currency = currency,
    );

    std::fs::write(&file_path, &initial_content)
        .map_err(|e| format!("Cannot write {}: {}", file_path.display(), e))?;

    let path_str = file_path.to_string_lossy().to_string();
    let loaded = load_journal(&path_str)?;
    let summary = make_summary(&loaded);

    let mut app_state = state.lock().map_err(|e| e.to_string())?;
    app_state.journal = Some(loaded);

    Ok(summary)
}

#[tauri::command]
pub async fn suggest_accounts(
    prefix: String,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<String>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    if prefix.is_empty() {
        Ok(loaded.ledger.account_names())
    } else {
        Ok(loaded.ledger.suggest_accounts(&prefix))
    }
}

#[tauri::command]
pub async fn suggest_descriptions(
    prefix: String,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<String>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    if prefix.is_empty() {
        Ok(loaded.ledger.descriptions())
    } else {
        Ok(loaded.ledger.suggest_descriptions(&prefix))
    }
}

#[tauri::command]
pub async fn suggest_payees(
    prefix: String,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<String>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    if prefix.is_empty() {
        Ok(loaded.ledger.descriptions())
    } else {
        Ok(loaded.ledger.suggest_payees(&prefix))
    }
}

#[tauri::command]
pub async fn update_transaction(
    index: usize,
    txn: NewTransaction,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let mut app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_mut().ok_or("No journal loaded")?;

    // Find the original transaction's span
    let original_txn = loaded
        .journal
        .items
        .iter()
        .filter_map(|item| match item {
            JournalItem::Transaction(t) => Some(t),
            _ => None,
        })
        .nth(index)
        .ok_or("Transaction not found")?;

    let span = original_txn.span.clone();

    // Build the new transaction AST (same logic as add_transaction)
    let date = chrono::NaiveDate::parse_from_str(&txn.date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid date: {}", e))?;

    let status = match txn.status.as_str() {
        "Cleared" | "cleared" | "*" => Status::Cleared,
        "Pending" | "pending" | "!" => Status::Pending,
        _ => Status::Unmarked,
    };

    let mut postings = Vec::new();
    for p in &txn.postings {
        let amount = if let Some(amt_str) = &p.amount {
            let quantity = rust_decimal::Decimal::from_str_exact(amt_str)
                .map_err(|e| format!("Invalid amount '{}': {}", amt_str, e))?;
            let commodity = p.commodity.clone().unwrap_or_default();
            let is_sym = commodity.len() == 1
                && "$€£¥₹₽₿₩₫₴₸₺₦₭".contains(commodity.chars().next().unwrap_or('x'));
            Some(PostingAmount {
                quantity,
                commodity: commodity.clone(),
                style: if is_sym {
                    AmountStyle { commodity_side: Side::Left, commodity_spaced: false, decimal_mark: '.', precision: 2 }
                } else if commodity.is_empty() {
                    AmountStyle::default()
                } else {
                    AmountStyle { commodity_side: Side::Right, commodity_spaced: true, decimal_mark: '.', precision: 2 }
                },
                cost: None,
            })
        } else {
            None
        };

        postings.push(Posting {
            span: SourceSpan { start: 0, end: 0, line: 0 },
            status: Status::Unmarked,
            account: AccountName::new(&p.account),
            amount,
            balance_assertion: None,
            comment: p.comment.as_ref().filter(|c| !c.is_empty()).map(|c| Comment { text: c.clone() }),
            tags: vec![],
            is_virtual: false,
            virtual_balanced: false,
        });
    }

    let ast_txn = Transaction {
        span: SourceSpan { start: 0, end: 0, line: 0 },
        date,
        secondary_date: None,
        status,
        code: None,
        description: txn.description,
        comment: txn.comment.as_ref().filter(|c| !c.is_empty()).map(|c| Comment { text: c.clone() }),
        tags: vec![],
        postings,
    };

    let new_text = writer::write_transaction(&ast_txn, &loaded.writer_config);

    // Patch the source text
    loaded.source_text = writer::patch_journal(&loaded.source_text, &[(span, new_text)]);

    // Write and re-resolve
    std::fs::write(&loaded.source_path, &loaded.source_text).map_err(|e| e.to_string())?;
    loaded.journal = hledger_parser::parse(&loaded.source_text).map_err(|e| e.to_string())?;
    loaded.ledger = Ledger::from_journal(&loaded.journal).map_err(|e| e.to_string())?;

    Ok(make_summary(loaded))
}

#[tauri::command]
pub async fn delete_transaction(
    index: usize,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let mut app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_mut().ok_or("No journal loaded")?;

    let original_txn = loaded
        .journal
        .items
        .iter()
        .filter_map(|item| match item {
            JournalItem::Transaction(t) => Some(t),
            _ => None,
        })
        .nth(index)
        .ok_or("Transaction not found")?;

    let span = original_txn.span.clone();

    loaded.source_text = writer::delete_from_journal(&loaded.source_text, &span);

    std::fs::write(&loaded.source_path, &loaded.source_text).map_err(|e| e.to_string())?;
    loaded.journal = hledger_parser::parse(&loaded.source_text).map_err(|e| e.to_string())?;
    loaded.ledger = Ledger::from_journal(&loaded.journal).map_err(|e| e.to_string())?;

    Ok(make_summary(loaded))
}

#[tauri::command]
pub async fn switch_journal(
    path: String,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let loaded = load_journal(&path)?;
    let summary = make_summary(&loaded);

    let mut app_state = state.lock().map_err(|e| e.to_string())?;
    app_state.journal = Some(loaded);

    Ok(summary)
}
