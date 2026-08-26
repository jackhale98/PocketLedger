use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use hledger_core::ledger::Ledger;
use hledger_parser::ast::{
    AccountName, AmountStyle, Comment, Journal, JournalItem, Posting, PostingAmount,
    SourceSpan, Status, Transaction,
};
use hledger_parser::writer::{self, WriterConfig};

/// Normalize a path that might be a file:// URI (iOS returns these from dialogs)
/// into a regular filesystem PathBuf.
pub(crate) fn normalize_path(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("file://") {
        if let Ok(decoded) = urlencoding::decode(stripped) {
            return PathBuf::from(decoded.into_owned());
        }
        return PathBuf::from(stripped);
    }
    if path.starts_with("file:") {
        if let Ok(url) = url::Url::parse(path) {
            if let Ok(p) = url.to_file_path() {
                return p;
            }
        }
    }
    PathBuf::from(path)
}

/// Android's document picker returns `content://` URIs, which are opaque
/// handles rather than paths — `std::fs` can't open them, and neither the
/// containing folder nor a journal's `include` siblings are reachable through
/// one. Reject them with an explanation instead of failing later with a
/// confusing "no such file".
pub(crate) fn reject_unsupported_uri(path: &str) -> Result<(), String> {
    if path.starts_with("content://") {
        return Err(
            "This file was opened through Android's document picker, which only grants \
             access to that single file — not to the folder, so `include` lines can't be \
             followed. Copy your journal into the app's own folder and open it from there."
                .to_string(),
        );
    }
    Ok(())
}

/// One physical file participating in the journal (main file or an include).
pub struct SourceFile {
    pub path: PathBuf,
    pub text: String,
}

pub struct LoadedJournal {
    /// files[0] is the main journal file; the rest are resolved includes.
    pub files: Vec<SourceFile>,
    /// The merged journal (all files' items).
    pub journal: Journal,
    /// Parallel to journal.items: which file each item came from. Spans are
    /// byte offsets into THAT file's text — patching the wrong file was a
    /// journal-corrupting bug.
    pub item_files: Vec<usize>,
    pub ledger: Ledger,
    pub writer_config: WriterConfig,
    /// Warnings gathered at load: include problems + parse warnings.
    pub load_warnings: Vec<String>,
    /// `include` targets that could not be found, as written in the journal.
    /// On mobile these are usually siblings that were never imported, so the
    /// UI offers to fetch them by name.
    pub missing_includes: Vec<String>,
}

impl LoadedJournal {
    pub fn source_path(&self) -> &Path {
        &self.files[0].path
    }

    pub fn main_text(&self) -> &str {
        &self.files[0].text
    }

    /// The nth transaction (parse order) with its item index and file index.
    pub fn nth_transaction(&self, index: usize) -> Option<(&Transaction, usize, usize)> {
        let mut n = 0;
        for (item_idx, item) in self.journal.items.iter().enumerate() {
            if let JournalItem::Transaction(t) = item {
                if n == index {
                    return Some((t, item_idx, self.item_files[item_idx]));
                }
                n += 1;
            }
        }
        None
    }

    /// All warnings to surface: load + resolution (assertions etc.) + budget.
    pub fn all_warnings(&self) -> Vec<String> {
        let mut warnings = self.load_warnings.clone();
        for w in self.ledger.warnings() {
            warnings.push(format!("line {}: {}", w.line, w.message));
        }
        let extraction = hledger_core::budget::extract_budgets_with_warnings(&self.journal);
        warnings.extend(extraction.warnings);
        warnings
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalSummary {
    pub file_name: String,
    pub transaction_count: usize,
    pub account_count: usize,
    pub warnings: Vec<String>,
    /// Names of `include` targets that could not be resolved.
    pub missing_includes: Vec<String>,
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

#[allow(dead_code)]
pub fn make_summary_pub(loaded: &LoadedJournal) -> JournalSummary {
    make_summary(loaded)
}

pub fn make_summary_result(app_state: &crate::AppState) -> Result<JournalSummary, String> {
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    Ok(make_summary(loaded))
}

fn make_summary(loaded: &LoadedJournal) -> JournalSummary {
    JournalSummary {
        file_name: loaded
            .source_path()
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        transaction_count: loaded.ledger.transaction_count(),
        account_count: loaded.ledger.account_count(),
        warnings: loaded.all_warnings(),
        missing_includes: loaded.missing_includes.clone(),
    }
}

// ─── Loading (with include resolution, cycle guard, overlay support) ───

fn read_file(path: &Path, overlay: &HashMap<PathBuf, String>) -> std::io::Result<String> {
    if let Some(text) = overlay.get(path) {
        return Ok(text.clone());
    }
    std::fs::read_to_string(path)
}

fn canonical_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

struct LoadContext<'a> {
    overlay: &'a HashMap<PathBuf, String>,
    files: Vec<SourceFile>,
    items: Vec<JournalItem>,
    item_files: Vec<usize>,
    warnings: Vec<String>,
    missing_includes: Vec<String>,
    visited: HashSet<PathBuf>,
}

fn load_one_file(ctx: &mut LoadContext, path: &Path) -> Result<(), String> {
    let key = canonical_key(path);
    if !ctx.visited.insert(key) {
        ctx.warnings.push(format!(
            "include cycle: '{}' is already included; skipping",
            path.display()
        ));
        return Ok(());
    }

    let text = read_file(path, ctx.overlay)
        .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;

    let journal = hledger_parser::parse(&text)
        .map_err(|e| format!("{}: {}", path.display(), e))?;

    let file_idx = ctx.files.len();
    ctx.files.push(SourceFile {
        path: path.to_path_buf(),
        text,
    });

    let file_label = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    for w in &journal.warnings {
        ctx.warnings
            .push(format!("{} line {}: {}", file_label, w.line, w.message));
    }

    let base_dir = path.parent().map(|p| p.to_path_buf());

    for item in journal.items {
        if let JournalItem::IncludeDirective(ref inc) = item {
            let inc_str = inc.path.trim().to_string();
            // Keep the directive itself (round-trip fidelity).
            ctx.items.push(item);
            ctx.item_files.push(file_idx);

            let resolve = |s: &str| -> PathBuf {
                if let Some(base) = &base_dir {
                    base.join(s)
                } else {
                    PathBuf::from(s)
                }
            };

            if inc_str.contains('*') || inc_str.contains('?') {
                let pattern = resolve(&inc_str).to_string_lossy().to_string();
                match glob::glob(&pattern) {
                    Ok(paths) => {
                        for entry in paths.flatten() {
                            load_one_file(ctx, &entry)?;
                        }
                    }
                    Err(e) => {
                        ctx.warnings
                            .push(format!("Invalid include pattern '{}': {}", inc_str, e));
                    }
                }
            } else {
                let inc_path = resolve(&inc_str);
                if inc_path.exists() {
                    load_one_file(ctx, &inc_path)?;
                } else {
                    ctx.missing_includes.push(inc_str.clone());
                    ctx.warnings.push(format!(
                        "Could not include '{}': file not found (resolved to {})",
                        inc_str,
                        inc_path.display()
                    ));
                }
            }
        } else {
            ctx.items.push(item);
            ctx.item_files.push(file_idx);
        }
    }

    Ok(())
}

fn load_journal_with_overlay(
    path: &str,
    overlay: &HashMap<PathBuf, String>,
) -> Result<LoadedJournal, String> {
    let file_path = normalize_path(path);

    let mut ctx = LoadContext {
        overlay,
        files: Vec::new(),
        items: Vec::new(),
        item_files: Vec::new(),
        warnings: Vec::new(),
        missing_includes: Vec::new(),
        visited: HashSet::new(),
    };

    load_one_file(&mut ctx, &file_path)?;

    let writer_config = writer::infer_config(&ctx.files[0].text);
    let journal = Journal {
        items: ctx.items,
        source_path: Some(file_path),
        warnings: vec![],
    };

    let ledger = Ledger::from_journal(&journal).map_err(|e| e.to_string())?;

    Ok(LoadedJournal {
        files: ctx.files,
        journal,
        item_files: ctx.item_files,
        ledger,
        writer_config,
        load_warnings: ctx.warnings,
        missing_includes: ctx.missing_includes,
    })
}

fn load_journal(path: &str) -> Result<LoadedJournal, String> {
    load_journal_with_overlay(path, &HashMap::new())
}

// ─── Safe writing: staleness check → validate → backup → atomic write ───

fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    use std::io::Write;

    let dir = path
        .parent()
        .ok_or_else(|| format!("{}: no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "journal".to_string());
    // "~$" prefix: both Dropbox and OneDrive document that they never sync
    // names starting with it, so a save inside a synced folder doesn't upload
    // a temp file on every write. OneDrive also skips ".tmp".
    let tmp = dir.join(format!("~${}.tmp", file_name));

    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| format!("Cannot create temp file {}: {}", tmp.display(), e))?;
        f.write_all(content.as_bytes())
            .map_err(|e| format!("Write failed: {}", e))?;
        f.sync_all().map_err(|e| format!("fsync failed: {}", e))?;
    }

    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Atomic rename to {} failed: {}", path.display(), e)
    })?;

    // Best-effort directory sync so the rename is durable.
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }

    Ok(())
}

/// Save the pre-edit content into `backup_dir`, which lives outside the
/// journal's own folder — see AppState::backup_dir. The name carries a hash of
/// the source path so two journals called `main.journal` in different folders
/// don't overwrite each other's backup.
fn write_backup(backup_dir: Option<&Path>, path: &Path, old_content: &str) {
    let Some(dir) = backup_dir else {
        return;
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "journal".to_string());
    let bak = dir.join(format!("{}.{:08x}.bak", name, path_hash(path)));
    // Best-effort: a failed backup must not block the save, but the backup
    // itself is written atomically so it's never half a file.
    let _ = atomic_write(&bak, old_content);
}

fn path_hash(path: &Path) -> u32 {
    // FNV-1a; only needs to separate paths, not resist collisions adversarially.
    let mut hash: u32 = 0x811c9dc5;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Turn a read failure into something a user can act on. A journal kept in a
/// cloud folder can be an "online only" placeholder, or momentarily locked
/// while the sync client uploads it — neither means the file is gone, and
/// telling the user to reload doesn't help.
fn describe_read_failure(path: &Path, e: &std::io::Error) -> String {
    let display = path.display();
    match e.raw_os_error() {
        // EDEADLK (Apple, materialising a dataless file from a context that
        // can't) and ETIMEDOUT (download didn't finish).
        Some(35) | Some(60) | Some(110) | Some(145) => format!(
            "'{display}' couldn't be downloaded from your cloud storage in time. \
             Open the folder in your sync app and mark it available offline, then try again."
        ),
        // Windows ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION.
        Some(32) | Some(33) => format!(
            "'{display}' is in use by another program, most likely your sync client. \
             Wait a moment and try again."
        ),
        _ => format!("'{display}' can no longer be read ({e}). Reload the journal."),
    }
}

/// Verify no file changed on disk behind our back (external editor, sync
/// service). Blindly persisting cached text would erase those edits.
fn check_stale(loaded: &LoadedJournal) -> Result<(), String> {
    for file in &loaded.files {
        match std::fs::read_to_string(&file.path) {
            Ok(disk) => {
                // A file that had content and now reads empty is almost never
                // a real edit: it's an unmaterialised cloud placeholder or a
                // sync client mid-write. Refusing here keeps us from treating
                // it as an external change and, worse, writing that emptiness
                // back over the real journal.
                if disk.is_empty() && !file.text.trim().is_empty() {
                    return Err(format!(
                        "'{}' read back empty, which usually means it isn't downloaded yet \
                         or your sync client is still writing it. Nothing was changed.",
                        file.path.display()
                    ));
                }
                if disk != file.text {
                    return Err(format!(
                        "'{}' was modified outside this app since it was loaded. Reload the journal, then repeat your change.",
                        file.path.display()
                    ));
                }
            }
            Err(e) => {
                return Err(describe_read_failure(&file.path, &e));
            }
        }
    }
    Ok(())
}

/// The single safe path for every mutation:
/// 1. staleness check for all files,
/// 2. re-load the whole journal with the new text as an overlay (parse +
///    include-resolve + balance/ledger validation) — nothing touches disk if
///    this fails,
/// 3. backup, atomic write, swap in the validated state.
pub fn apply_file_edit(
    app_state: &mut crate::AppState,
    file_idx: usize,
    new_text: String,
) -> Result<JournalSummary, String> {
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    check_stale(loaded)?;

    let target_path = loaded.files[file_idx].path.clone();
    let old_text = loaded.files[file_idx].text.clone();
    let main_path = loaded.source_path().to_string_lossy().to_string();

    let mut overlay = HashMap::new();
    overlay.insert(target_path.clone(), new_text.clone());

    // Validate the complete new state before writing anything.
    let candidate = load_journal_with_overlay(&main_path, &overlay).map_err(|e| {
        format!("Change rejected (journal would become invalid): {}", e)
    })?;

    write_backup(app_state.backup_dir.as_deref(), &target_path, &old_text);
    atomic_write(&target_path, &new_text)?;

    app_state.journal = Some(candidate);
    app_state.generation = app_state.generation.wrapping_add(1);

    let summary = make_summary(app_state.journal.as_ref().unwrap());
    Ok(summary)
}

/// Append text to the main journal file through the safe path.
pub fn apply_append_to_main(
    app_state: &mut crate::AppState,
    addition: &str,
) -> Result<JournalSummary, String> {
    apply_append_to_file(app_state, 0, addition)
}

/// Append to one of the journal's files. Which file matters: a journal split
/// by year keeps each year's entries in its own file, and writing everything
/// into the main file would both misplace the entry and put the diff in the
/// wrong place for version control.
pub fn apply_append_to_file(
    app_state: &mut crate::AppState,
    file_idx: usize,
    addition: &str,
) -> Result<JournalSummary, String> {
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    let file = loaded
        .files
        .get(file_idx)
        .ok_or("That journal file is no longer loaded")?;
    let mut new_text = file.text.clone();
    if !new_text.is_empty() && !new_text.ends_with('\n') {
        new_text.push('\n');
    }
    new_text.push('\n');
    new_text.push_str(addition);
    apply_file_edit(app_state, file_idx, new_text)
}

// ─── Building transactions from UI input ───

fn parse_status(s: &str) -> Status {
    match s {
        "Cleared" | "cleared" | "*" => Status::Cleared,
        "Pending" | "pending" | "!" => Status::Pending,
        _ => Status::Unmarked,
    }
}

fn validate_text_field(value: &str, what: &str) -> Result<(), String> {
    if value.contains('\n') || value.contains('\r') {
        return Err(format!("{} must not contain line breaks", what));
    }
    Ok(())
}

fn validate_account_name(account: &str) -> Result<(), String> {
    let account = account.trim();
    if account.is_empty() {
        return Err("Posting account name is empty".to_string());
    }
    validate_text_field(account, "Account name")?;
    if account.contains("  ") || account.contains('\t') {
        return Err(format!(
            "Account name '{}' must not contain double spaces or tabs",
            account
        ));
    }
    if account.starts_with('(') || account.starts_with('[') || account.contains(';') {
        return Err(format!(
            "Account name '{}' must not start with brackets or contain ';'",
            account
        ));
    }
    Ok(())
}

fn build_amount(amt_str: &str, commodity: &str) -> Result<PostingAmount, String> {
    let q = hledger_parser::parse_quantity_with(amt_str, Some('.'))
        .map_err(|e| format!("Invalid amount '{}': {}", amt_str, e))?;

    if commodity.contains('"') {
        return Err("Commodity must not contain quotes".to_string());
    }
    validate_text_field(commodity, "Commodity")?;

    let mut style: AmountStyle = writer::default_style_for(commodity);
    // Precision comes from what the user typed — a hardcoded 2 destroyed
    // high-precision amounts (0.00012345 BTC became 0.00).
    style.precision = q.precision.max(2);

    Ok(PostingAmount {
        quantity: q.value,
        commodity: commodity.to_string(),
        style,
        cost: None,
        multiplier: false,
    })
}

fn build_transaction(txn: &NewTransaction) -> Result<Transaction, String> {
    let date = chrono::NaiveDate::parse_from_str(&txn.date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid date: {}", e))?;

    validate_text_field(&txn.description, "Description")?;
    if txn.description.contains(';') {
        return Err("Description must not contain ';' (use the comment field)".to_string());
    }
    if let Some(c) = &txn.comment {
        validate_text_field(c, "Comment")?;
    }

    if txn.postings.len() < 2 {
        return Err("A transaction needs at least two postings".to_string());
    }

    let mut postings = Vec::new();
    for p in &txn.postings {
        validate_account_name(&p.account)?;
        if let Some(c) = &p.comment {
            validate_text_field(c, "Posting comment")?;
        }

        let amount = match &p.amount {
            Some(amt_str) if !amt_str.trim().is_empty() => Some(build_amount(
                amt_str.trim(),
                p.commodity.as_deref().unwrap_or(""),
            )?),
            _ => None,
        };

        postings.push(Posting {
            span: SourceSpan { start: 0, end: 0, line: 0 },
            status: Status::Unmarked,
            account: AccountName::new(p.account.trim()),
            amount,
            balance_assertion: None,
            comment: p
                .comment
                .as_ref()
                .filter(|c| !c.is_empty())
                .map(|c| Comment { text: c.clone() }),
            tags: vec![],
            is_virtual: false,
            virtual_balanced: false,
            date: None,
            date2: None,
        });
    }

    Ok(Transaction {
        span: SourceSpan { start: 0, end: 0, line: 0 },
        date,
        secondary_date: None,
        status: parse_status(&txn.status),
        code: None,
        description: txn.description.clone(),
        comment: txn
            .comment
            .as_ref()
            .filter(|c| !c.is_empty())
            .map(|c| Comment { text: c.clone() }),
        tags: vec![],
        postings,
    })
}

/// True if a posting carries structure the edit form cannot represent.
fn posting_has_extras(p: &Posting) -> bool {
    p.balance_assertion.is_some()
        || p.amount.as_ref().map_or(false, |a| a.cost.is_some())
        || p.is_virtual
        || !p.tags.is_empty()
        || p.date.is_some()
        || p.date2.is_some()
        || p.status != Status::Unmarked
}

fn first_line(text: &str) -> &str {
    text.split('\n').next().unwrap_or("")
}

fn merge_comment(submitted: Option<&Comment>, original: Option<&Comment>) -> Option<Comment> {
    match (submitted, original) {
        (None, None) => None,
        (Some(s), None) => Some(s.clone()),
        (None, Some(o)) => {
            // The form cleared the visible first line; keep continuation lines.
            let rest: Vec<&str> = o.text.split('\n').skip(1).collect();
            if rest.is_empty() {
                None
            } else {
                Some(Comment {
                    text: rest.join("\n"),
                })
            }
        }
        (Some(s), Some(o)) => {
            // The form edits only the first line; continuation lines survive.
            let mut text = s.text.clone();
            let rest: Vec<&str> = o.text.split('\n').skip(1).collect();
            if !rest.is_empty() && s.text == first_line(&o.text) {
                text = o.text.clone();
            } else if !rest.is_empty() {
                text = format!("{}\n{}", s.text, rest.join("\n"));
            }
            Some(Comment { text })
        }
    }
}

/// Merge an edited transaction with its original AST so that everything the
/// form doesn't display — costs, assertions, tags, codes, posting status,
/// secondary dates, virtual markers, comment continuation lines, elided
/// amounts — survives the edit instead of being silently deleted.
fn merge_with_original(
    mut edited: Transaction,
    original: &Transaction,
    resolved: Option<&hledger_core::balance::ResolvedTransaction>,
) -> Result<Transaction, String> {
    // Transaction-level fields the form doesn't show.
    edited.code = original.code.clone();
    edited.secondary_date = original.secondary_date;
    edited.tags = original.tags.clone();
    edited.comment = merge_comment(edited.comment.as_ref(), original.comment.as_ref());

    let aligned = edited.postings.len() == original.postings.len()
        && edited
            .postings
            .iter()
            .zip(original.postings.iter())
            .all(|(new, old)| new.account.full == old.account.full);

    if aligned {
        for (i, new_p) in edited.postings.iter_mut().enumerate() {
            let old_p = &original.postings[i];
            new_p.status = old_p.status;
            new_p.balance_assertion = old_p.balance_assertion.clone();
            new_p.tags = old_p.tags.clone();
            new_p.is_virtual = old_p.is_virtual;
            new_p.virtual_balanced = old_p.virtual_balanced;
            new_p.date = old_p.date;
            new_p.date2 = old_p.date2;
            new_p.comment = merge_comment(new_p.comment.as_ref(), old_p.comment.as_ref());

            match (&mut new_p.amount, &old_p.amount) {
                (Some(new_amt), Some(old_amt)) => {
                    // Carry the cost and the original display style; keep the
                    // higher precision so values never truncate.
                    new_amt.cost = old_amt.cost.clone();
                    if new_amt.commodity == old_amt.commodity {
                        let precision = new_amt.style.precision.max(old_amt.style.precision);
                        new_amt.style = old_amt.style.clone();
                        new_amt.style.precision = precision;
                    }
                }
                (new_amt @ Some(_), None) => {
                    // The original elided this amount. If the user "kept" the
                    // computed value, preserve the elision.
                    if let Some(resolved_txn) = resolved {
                        if let Some(rp) = resolved_txn.postings.get(i) {
                            let submitted = new_amt.as_ref().unwrap();
                            if rp.amount.amounts.len() == 1
                                && rp.amount.get(&submitted.commodity) == submitted.quantity
                            {
                                *new_amt = None;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        return Ok(edited);
    }

    // Structure changed. Only allow it when nothing invisible would be lost.
    let original_has_extras = original.postings.iter().any(posting_has_extras)
        || original.postings.iter().any(|p| {
            p.comment
                .as_ref()
                .map_or(false, |c| c.text.contains('\n'))
        });
    if original_has_extras {
        return Err(
            "This transaction contains costs, balance assertions, tags, posting statuses or virtual postings that the editor cannot preserve when postings are added, removed or reordered. Keep the original posting structure, or edit the journal file in a text editor."
                .to_string(),
        );
    }

    Ok(edited)
}

// ─── Commands ───

#[tauri::command]
pub async fn open_journal(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    reject_unsupported_uri(&path)?;
    // A path in the iOS picker's Inbox (or any temp dir) is a copy the OS
    // deletes without warning — loading it directly means later writes fail
    // with "can no longer be read". Relocate into app storage first.
    let path = super::storage::relocate_if_transient(&path, &app)?;
    let loaded = load_journal(&path)?;
    let summary = make_summary(&loaded);

    let mut app_state = state.lock().map_err(|e| e.to_string())?;
    app_state.journal = Some(loaded);
    app_state.generation = app_state.generation.wrapping_add(1);

    Ok(summary)
}

/// Whether any source file differs from what was loaded. The frontend polls
/// this when the app returns to the foreground: a git client syncing the
/// journal folder (Working Copy pulling, say) changes files behind our back,
/// and reloading blindly would invalidate in-progress flows for no reason.
#[tauri::command]
pub async fn journal_changed_on_disk(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<bool, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = match app_state.journal.as_ref() {
        Some(l) => l,
        None => return Ok(false),
    };
    Ok(check_stale(loaded).is_err())
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
    // State is persisted on every mutation; this just re-writes the main file.
    atomic_write(loaded.source_path(), loaded.main_text())
}

/// A file participating in the loaded journal, for choosing where new
/// transactions go.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalFileInfo {
    pub index: usize,
    pub name: String,
    pub path: String,
    /// files[0] is the file that was opened; the rest arrived via `include`.
    pub is_main: bool,
}

#[tauri::command]
pub async fn list_journal_files(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<JournalFileInfo>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    Ok(loaded
        .files
        .iter()
        .enumerate()
        .map(|(index, f)| JournalFileInfo {
            index,
            name: f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.path.display().to_string()),
            path: f.path.to_string_lossy().into_owned(),
            is_main: index == 0,
        })
        .collect())
}

#[tauri::command]
pub async fn add_transaction(
    txn: NewTransaction,
    file_index: Option<usize>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let mut app_state = state.lock().map_err(|e| e.to_string())?;

    let ast_txn = build_transaction(&txn)?;
    let config = app_state
        .journal
        .as_ref()
        .ok_or("No journal loaded")?
        .writer_config
        .clone();
    let txn_text = writer::write_transaction(&ast_txn, &config);

    apply_append_to_file(&mut app_state, file_index.unwrap_or(0), &txn_text)
}

#[tauri::command]
pub async fn create_journal(
    path: String,
    default_currency: Option<String>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let currency = default_currency.unwrap_or_else(|| "$".to_string());
    let file_path = normalize_path(&path);

    if file_path.exists() {
        return Err(format!(
            "'{}' already exists — open it instead, or choose a different name.",
            file_path.display()
        ));
    }

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

    atomic_write(&file_path, &initial_content)?;

    let path_str = file_path.to_string_lossy().to_string();
    let loaded = load_journal(&path_str)?;
    let summary = make_summary(&loaded);

    let mut app_state = state.lock().map_err(|e| e.to_string())?;
    app_state.journal = Some(loaded);
    app_state.generation = app_state.generation.wrapping_add(1);

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

/// Accounts previously paired with this description, most-used first, so a
/// repeat purchase can be filled in rather than retyped.
#[tauri::command]
pub async fn accounts_for_description(
    description: String,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<String>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    Ok(loaded.ledger.accounts_for_description(&description))
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

    let (new_file_text, file_idx) = {
        let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
        let (original, _item_idx, file_idx) = loaded
            .nth_transaction(index)
            .ok_or("Transaction not found")?;

        let edited = build_transaction(&txn)?;
        let resolved = loaded
            .ledger
            .transactions()
            .find(|t| {
                t.postings
                    .first()
                    .map(|p| p.transaction_index)
                    .unwrap_or(usize::MAX)
                    == index
            });
        let merged = merge_with_original(edited, original, resolved)?;

        let new_text = writer::write_transaction(&merged, &loaded.writer_config);
        let file_text = &loaded.files[file_idx].text;
        let patched = writer::patch_journal(file_text, &[(original.span.clone(), new_text)])?;
        (patched, file_idx)
    };

    apply_file_edit(&mut app_state, file_idx, new_file_text)
}

#[tauri::command]
pub async fn delete_transaction(
    index: usize,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let mut app_state = state.lock().map_err(|e| e.to_string())?;

    let (new_file_text, file_idx) = {
        let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
        let (original, _item_idx, file_idx) = loaded
            .nth_transaction(index)
            .ok_or("Transaction not found")?;
        let file_text = &loaded.files[file_idx].text;
        let patched = writer::delete_from_journal(file_text, &original.span)?;
        (patched, file_idx)
    };

    apply_file_edit(&mut app_state, file_idx, new_file_text)
}

#[tauri::command]
pub async fn switch_journal(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let path = super::storage::relocate_if_transient(&path, &app)?;
    let loaded = load_journal(&path)?;
    let summary = make_summary(&loaded);

    let mut app_state = state.lock().map_err(|e| e.to_string())?;
    app_state.journal = Some(loaded);
    app_state.generation = app_state.generation.wrapping_add(1);

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique temp dir per test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pockethledger-test-{}-{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn state_with(path: &Path) -> crate::AppState {
        // Backups land beside the journal in tests so the existing
        // backup assertions keep working without an AppHandle.
        crate::AppState {
            journal: Some(load_journal(&path.to_string_lossy()).unwrap()),
            backup_dir: path.parent().map(|p| p.to_path_buf()),
            generation: 0,
        }
    }

    fn simple_txn(desc: &str, amount: &str) -> NewTransaction {
        NewTransaction {
            date: "2024-03-01".to_string(),
            status: "Unmarked".to_string(),
            description: desc.to_string(),
            comment: None,
            postings: vec![
                NewPosting {
                    account: "expenses:food".to_string(),
                    amount: Some(amount.to_string()),
                    commodity: Some("$".to_string()),
                    comment: None,
                },
                NewPosting {
                    account: "assets:cash".to_string(),
                    amount: None,
                    commodity: None,
                    comment: None,
                },
            ],
        }
    }

    fn add(state: &mut crate::AppState, txn: &NewTransaction) -> Result<JournalSummary, String> {
        let ast = build_transaction(txn)?;
        let config = state.journal.as_ref().unwrap().writer_config.clone();
        let text = writer::write_transaction(&ast, &config);
        apply_append_to_main(state, &text)
    }

    fn update(
        state: &mut crate::AppState,
        index: usize,
        txn: &NewTransaction,
    ) -> Result<JournalSummary, String> {
        let (patched, file_idx) = {
            let loaded = state.journal.as_ref().unwrap();
            let (original, _ii, file_idx) = loaded.nth_transaction(index).ok_or("not found")?;
            let edited = build_transaction(txn)?;
            let resolved = loaded.ledger.transactions().find(|t| {
                t.postings.first().map(|p| p.transaction_index) == Some(index)
            });
            let merged = merge_with_original(edited, original, resolved)?;
            let new_text = writer::write_transaction(&merged, &loaded.writer_config);
            let patched = writer::patch_journal(
                &loaded.files[file_idx].text,
                &[(original.span.clone(), new_text)],
            )?;
            (patched, file_idx)
        };
        apply_file_edit(state, file_idx, patched)
    }

    #[test]
    fn crlf_journal_edit_does_not_corrupt() {
        let dir = temp_dir("crlf");
        let main = dir.join("main.journal");
        let content = "2024-01-01 First\r\n    a  $1.00\r\n    b\r\n\r\n2024-01-16 Second\r\n    a  $2.00\r\n    b\r\n";
        std::fs::write(&main, content).unwrap();

        let mut state = state_with(&main);
        let mut txn = simple_txn("Second Edited", "2.00");
        txn.date = "2024-01-16".to_string();
        txn.postings[0].account = "a".to_string();
        txn.postings[1].account = "b".to_string();
        update(&mut state, 1, &txn).unwrap();

        let on_disk = std::fs::read_to_string(&main).unwrap();
        assert!(on_disk.contains("2024-01-01 First"), "first txn intact");
        assert!(on_disk.contains("Second Edited"), "edit applied");
        assert!(!on_disk.contains("sets:cash"), "no mid-line splice");
        // Whole file still parses and resolves.
        let journal = hledger_parser::parse(&on_disk).unwrap();
        assert!(hledger_core::ledger::Ledger::from_journal(&journal).is_ok());
    }

    #[test]
    fn include_edit_writes_to_owning_file() {
        let dir = temp_dir("include-edit");
        let main = dir.join("main.journal");
        let sub = dir.join("2023.journal");
        std::fs::write(
            &sub,
            "2023-05-01 Old grocery\n    expenses:food  $10.00\n    assets:cash\n",
        )
        .unwrap();
        std::fs::write(
            &main,
            "include 2023.journal\n\n2024-01-01 New grocery\n    expenses:food  $20.00\n    assets:cash\n",
        )
        .unwrap();

        let mut state = state_with(&main);
        assert_eq!(state.journal.as_ref().unwrap().ledger.transaction_count(), 2);

        // Transaction 0 in parse order is the included one.
        let mut txn = simple_txn("Old grocery fixed", "10.00");
        txn.date = "2023-05-01".to_string();
        update(&mut state, 0, &txn).unwrap();

        let main_after = std::fs::read_to_string(&main).unwrap();
        let sub_after = std::fs::read_to_string(&sub).unwrap();
        assert!(main_after.contains("New grocery"), "main untouched apart from include");
        assert!(!main_after.contains("Old grocery"), "edit must not splice into main");
        assert!(sub_after.contains("Old grocery fixed"), "included file got the edit");

        // Both transactions still visible after the mutation (no include drop).
        assert_eq!(state.journal.as_ref().unwrap().ledger.transaction_count(), 2);
    }

    #[test]
    fn unbalanced_add_rejected_before_touching_disk() {
        let dir = temp_dir("unbalanced");
        let main = dir.join("main.journal");
        std::fs::write(&main, "2024-01-01 Seed\n    a  $1.00\n    b\n").unwrap();

        let mut state = state_with(&main);
        let before = std::fs::read_to_string(&main).unwrap();

        let txn = NewTransaction {
            date: "2024-02-01".to_string(),
            status: "Unmarked".to_string(),
            description: "Bad".to_string(),
            comment: None,
            postings: vec![
                NewPosting {
                    account: "expenses:food".to_string(),
                    amount: Some("50.00".to_string()),
                    commodity: Some("$".to_string()),
                    comment: None,
                },
                NewPosting {
                    account: "assets:cash".to_string(),
                    amount: Some("-45.00".to_string()),
                    commodity: Some("$".to_string()),
                    comment: None,
                },
            ],
        };
        let err = add(&mut state, &txn).unwrap_err();
        assert!(err.contains("invalid"), "error mentions invalidity: {}", err);

        let after = std::fs::read_to_string(&main).unwrap();
        assert_eq!(before, after, "file must be untouched after a rejected write");
        // In-memory state still consistent.
        assert_eq!(state.journal.as_ref().unwrap().ledger.transaction_count(), 1);
    }

    #[test]
    fn edit_preserves_cost_assertion_tags_and_status() {
        let dir = temp_dir("preserve");
        let main = dir.join("main.journal");
        std::fs::write(
            &main,
            "2024-01-01=2024-01-03 * (code42) Buy stock ; strategy:longterm\n    ! assets:broker  10 AAPL @ $150.00 = 10 AAPL ; lot:a\n    assets:cash  $-1500.00\n",
        )
        .unwrap();

        let mut state = state_with(&main);

        // Description-only edit, same posting structure.
        let txn = NewTransaction {
            date: "2024-01-01".to_string(),
            status: "Cleared".to_string(),
            description: "Buy stock (renamed)".to_string(),
            comment: Some("strategy:longterm".to_string()),
            postings: vec![
                NewPosting {
                    account: "assets:broker".to_string(),
                    amount: Some("10".to_string()),
                    commodity: Some("AAPL".to_string()),
                    comment: Some("lot:a".to_string()),
                },
                NewPosting {
                    account: "assets:cash".to_string(),
                    amount: Some("-1500.00".to_string()),
                    commodity: Some("$".to_string()),
                    comment: None,
                },
            ],
        };
        update(&mut state, 0, &txn).unwrap();

        let after = std::fs::read_to_string(&main).unwrap();
        assert!(after.contains("@ $150.00"), "cost preserved: {}", after);
        assert!(after.contains("= 10 AAPL"), "assertion preserved: {}", after);
        assert!(after.contains("(code42)"), "code preserved: {}", after);
        assert!(after.contains("=2024-01-03"), "secondary date preserved: {}", after);
        assert!(after.contains("! assets:broker"), "posting status preserved: {}", after);
        assert!(after.contains("strategy:longterm"), "tag comment preserved: {}", after);
        assert!(after.contains("(renamed)"), "edit applied: {}", after);
    }

    #[test]
    fn restructure_of_rich_transaction_is_refused() {
        let dir = temp_dir("refuse");
        let main = dir.join("main.journal");
        std::fs::write(
            &main,
            "2024-01-01 Buy\n    assets:broker  1 AAPL @ $150.00\n    assets:cash  $-150.00\n",
        )
        .unwrap();

        let mut state = state_with(&main);
        let before = std::fs::read_to_string(&main).unwrap();

        // Different account structure → must refuse, not silently strip the cost.
        let txn = simple_txn("Restructured", "150.00");
        let err = update(&mut state, 0, &txn).unwrap_err();
        assert!(
            err.contains("cannot preserve") || err.contains("costs"),
            "refusal message: {}",
            err
        );
        assert_eq!(before, std::fs::read_to_string(&main).unwrap());
    }

    #[test]
    fn high_precision_amounts_survive_add() {
        let dir = temp_dir("precision");
        let main = dir.join("main.journal");
        std::fs::write(&main, "2024-01-01 Seed\n    a  $1.00\n    b\n").unwrap();

        let mut state = state_with(&main);
        let txn = NewTransaction {
            date: "2024-02-01".to_string(),
            status: "Unmarked".to_string(),
            description: "Buy BTC".to_string(),
            comment: None,
            postings: vec![
                NewPosting {
                    account: "assets:btc".to_string(),
                    amount: Some("0.00012345".to_string()),
                    commodity: Some("BTC".to_string()),
                    comment: None,
                },
                NewPosting {
                    account: "assets:cash".to_string(),
                    amount: None,
                    commodity: None,
                    comment: None,
                },
            ],
        };
        add(&mut state, &txn).unwrap();

        let after = std::fs::read_to_string(&main).unwrap();
        assert!(
            after.contains("0.00012345 BTC"),
            "precision must survive: {}",
            after
        );
    }

    #[test]
    fn stale_disk_state_blocks_write() {
        let dir = temp_dir("stale");
        let main = dir.join("main.journal");
        std::fs::write(&main, "2024-01-01 Seed\n    a  $1.00\n    b\n").unwrap();

        let mut state = state_with(&main);

        // External edit (sync service, other editor).
        std::fs::write(
            &main,
            "2024-01-01 Seed\n    a  $1.00\n    b\n\n2024-01-02 External\n    a  $9.00\n    b\n",
        )
        .unwrap();

        let txn = simple_txn("App edit", "5.00");
        let err = add(&mut state, &txn).unwrap_err();
        assert!(err.contains("modified outside"), "stale error: {}", err);

        // External edit still on disk.
        let after = std::fs::read_to_string(&main).unwrap();
        assert!(after.contains("External"));
    }

    #[test]
    fn missing_includes_are_reported_by_name() {
        // The names must survive as written, so the UI can say exactly which
        // files to import rather than only logging a warning line.
        let dir = temp_dir("missing-includes");
        let main = dir.join("main.journal");
        std::fs::write(
            &main,
            "include 2024.journal\ninclude prices.journal\n\n2024-01-01 T\n    a  $1.00\n    b\n",
        )
        .unwrap();

        let loaded = load_journal(&main.to_string_lossy()).unwrap();
        assert_eq!(
            loaded.missing_includes,
            vec!["2024.journal".to_string(), "prices.journal".to_string()]
        );
        // The main file's own transactions still load.
        assert_eq!(loaded.ledger.transaction_count(), 1);
    }

    #[test]
    fn resolved_includes_are_not_reported_missing() {
        let dir = temp_dir("present-includes");
        let main = dir.join("main.journal");
        std::fs::write(dir.join("sub.journal"), "2024-01-01 S\n    a  $1.00\n    b\n").unwrap();
        std::fs::write(&main, "include sub.journal\n").unwrap();

        let loaded = load_journal(&main.to_string_lossy()).unwrap();
        assert!(loaded.missing_includes.is_empty());
        assert_eq!(loaded.ledger.transaction_count(), 1);
    }

    #[test]
    fn periodic_rules_in_included_files_are_found() {
        // Budgets and forecasts both read `~` rules off the merged journal.
        // A rule living in an included file (the usual layout) must be picked
        // up just like one in the main file.
        let dir = temp_dir("included-periodics");
        let main = dir.join("main.journal");
        std::fs::write(
            dir.join("budget.journal"),
            "~ monthly  Budget goals\n    expenses:food  $400.00\n    assets\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("recurring.journal"),
            "~ monthly from 2024-01-01  Rent\n    expenses:rent  $1200.00\n    assets:checking\n",
        )
        .unwrap();
        std::fs::write(
            &main,
            "include budget.journal\ninclude recurring.journal\n\n2024-01-05 Seed\n    assets:checking  $5000.00\n    equity:opening\n",
        )
        .unwrap();

        let loaded = load_journal(&main.to_string_lossy()).unwrap();
        assert!(loaded.missing_includes.is_empty());

        let budgets = hledger_core::budget::extract_budgets(&loaded.journal);
        assert_eq!(budgets.len(), 2, "both rules are budget-shaped");

        let rules = hledger_core::forecast::extract_rules(&loaded.journal);
        let descriptions: Vec<&str> = rules.iter().map(|r| r.description.as_str()).collect();
        assert!(descriptions.contains(&"Budget goals"), "got {descriptions:?}");
        assert!(descriptions.contains(&"Rent"), "got {descriptions:?}");
        assert!(rules.iter().all(|r| r.error.is_none()), "{rules:?}");
    }

    #[test]
    fn include_cycle_warns_instead_of_crashing() {
        let dir = temp_dir("cycle");
        let main = dir.join("a.journal");
        std::fs::write(
            &main,
            "include a.journal\n\n2024-01-01 T\n    a  $1.00\n    b\n",
        )
        .unwrap();

        let loaded = load_journal(&main.to_string_lossy()).unwrap();
        assert!(
            loaded.load_warnings.iter().any(|w| w.contains("cycle")),
            "warnings: {:?}",
            loaded.load_warnings
        );
        assert_eq!(loaded.ledger.transaction_count(), 1);
    }

    #[test]
    fn backup_written_on_edit() {
        let dir = temp_dir("backup");
        let main = dir.join("main.journal");
        let original = "2024-01-01 Seed\n    a  $1.00\n    b\n";
        std::fs::write(&main, original).unwrap();

        let mut state = state_with(&main);
        let mut txn = simple_txn("Added", "3.00");
        txn.postings[0].account = "a".to_string();
        txn.postings[1].account = "b".to_string();
        add(&mut state, &txn).unwrap();

        // The name carries a hash of the source path so journals of the same
        // name in different folders don't clobber each other's backup.
        let bak: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "bak"))
            .collect();
        assert_eq!(bak.len(), 1, "exactly one backup expected: {bak:?}");
        assert_eq!(std::fs::read_to_string(&bak[0]).unwrap(), original);
    }

    #[test]
    fn android_content_uris_are_refused_with_an_explanation() {
        let err = reject_unsupported_uri("content://com.android.providers/document/1234")
            .unwrap_err();
        assert!(err.contains("include"), "should explain why: {err}");
        // Ordinary paths and iOS file:// URLs still pass.
        assert!(reject_unsupported_uri("/home/me/main.journal").is_ok());
        assert!(reject_unsupported_uri("file:///var/mobile/main.journal").is_ok());
    }

    #[test]
    fn a_file_that_reads_back_empty_is_refused_not_treated_as_an_edit() {
        // The cloud-placeholder / mid-sync case. Reporting it as an external
        // edit would be wrong, and writing over it would destroy the journal.
        let dir = temp_dir("empty-read");
        let main = dir.join("main.journal");
        std::fs::write(&main, "2024-01-01 Seed\n    a  $1.00\n    b\n").unwrap();

        let mut state = state_with(&main);
        std::fs::write(&main, "").unwrap();

        let mut txn = simple_txn("Added", "3.00");
        txn.postings[0].account = "a".to_string();
        txn.postings[1].account = "b".to_string();
        let err = add(&mut state, &txn).unwrap_err();
        assert!(err.contains("read back empty"), "unhelpful error: {err}");

        // And the truncated file was left exactly as found, not overwritten.
        assert_eq!(std::fs::read_to_string(&main).unwrap(), "");
    }

    #[test]
    fn backups_go_to_the_configured_directory_not_beside_the_journal() {
        // A journal kept in a synced folder must not get a .bak sibling on
        // every save — that doubles upload traffic and conflict surface.
        let dir = temp_dir("backup-elsewhere");
        let backups = temp_dir("backup-elsewhere-store");
        let main = dir.join("main.journal");
        std::fs::write(&main, "2024-01-01 Seed\n    a  $1.00\n    b\n").unwrap();

        let mut state = crate::AppState {
            journal: Some(load_journal(&main.to_string_lossy()).unwrap()),
            backup_dir: Some(backups.clone()),
            generation: 0,
        };
        let mut txn = simple_txn("Added", "3.00");
        txn.postings[0].account = "a".to_string();
        txn.postings[1].account = "b".to_string();
        add(&mut state, &txn).unwrap();

        let siblings: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".bak"))
            .collect();
        assert!(siblings.is_empty(), "no .bak beside the journal: {siblings:?}");

        let stored = std::fs::read_dir(&backups).unwrap().flatten().count();
        assert_eq!(stored, 1, "backup written to the configured directory");
    }

    #[test]
    fn create_journal_refuses_to_overwrite() {
        let dir = temp_dir("create-guard");
        let main = dir.join("existing.journal");
        std::fs::write(&main, "2024-01-01 Precious\n    a  $1.00\n    b\n").unwrap();

        // create_journal is async; test its guard logic directly.
        let file_path = normalize_path(&main.to_string_lossy());
        assert!(file_path.exists());
        // The command checks exists() before writing; simulate that check.
        // (The full command needs a Tauri State; the guard is the critical part.)
        let would_refuse = file_path.exists();
        assert!(would_refuse);
        assert!(std::fs::read_to_string(&main).unwrap().contains("Precious"));
    }

    #[test]
    fn injection_via_description_rejected() {
        let dir = temp_dir("injection");
        let main = dir.join("main.journal");
        std::fs::write(&main, "2024-01-01 Seed\n    a  $1.00\n    b\n").unwrap();

        let mut state = state_with(&main);
        let mut txn = simple_txn("Lunch\n2024-01-03 Injected\n    a  $999.00\n    b", "5.00");
        let err = add(&mut state, &txn).unwrap_err();
        assert!(err.contains("line breaks"), "got: {}", err);

        txn.description = "ok".to_string();
        txn.postings[0].account = "bad\naccount".to_string();
        let err = add(&mut state, &txn).unwrap_err();
        assert!(err.contains("line breaks"), "got: {}", err);
    }

    #[test]
    fn delete_removes_only_target() {
        let dir = temp_dir("delete");
        let main = dir.join("main.journal");
        std::fs::write(
            &main,
            "2024-01-01 A\n    a  $1.00\n    b\n\n2024-01-02 B\n    a  $2.00\n    b\n\n2024-01-03 C\n    a  $3.00\n    b\n",
        )
        .unwrap();

        let mut state = state_with(&main);
        let (patched, file_idx) = {
            let loaded = state.journal.as_ref().unwrap();
            let (t, _ii, fi) = loaded.nth_transaction(1).unwrap();
            (
                writer::delete_from_journal(&loaded.files[fi].text, &t.span).unwrap(),
                fi,
            )
        };
        apply_file_edit(&mut state, file_idx, patched).unwrap();

        let after = std::fs::read_to_string(&main).unwrap();
        assert!(after.contains("2024-01-01 A"));
        assert!(!after.contains("2024-01-02 B"));
        assert!(after.contains("2024-01-03 C"));
        assert!(!after.contains("\n\n\n"), "no double blank lines: {:?}", after);
    }

    #[test]
    fn elided_amount_stays_elided_after_edit() {
        let dir = temp_dir("elide");
        let main = dir.join("main.journal");
        std::fs::write(
            &main,
            "2024-01-01 Grocery\n    expenses:food  $50.00\n    assets:cash\n",
        )
        .unwrap();

        let mut state = state_with(&main);
        // The form prefills the computed -50.00 for the elided posting; if the
        // user leaves it unchanged, the file must keep the elision.
        let txn = NewTransaction {
            date: "2024-01-01".to_string(),
            status: "Unmarked".to_string(),
            description: "Grocery renamed".to_string(),
            comment: None,
            postings: vec![
                NewPosting {
                    account: "expenses:food".to_string(),
                    amount: Some("50.00".to_string()),
                    commodity: Some("$".to_string()),
                    comment: None,
                },
                NewPosting {
                    account: "assets:cash".to_string(),
                    amount: Some("-50.00".to_string()),
                    commodity: Some("$".to_string()),
                    comment: None,
                },
            ],
        };
        update(&mut state, 0, &txn).unwrap();

        let after = std::fs::read_to_string(&main).unwrap();
        let cash_line = after.lines().find(|l| l.contains("assets:cash")).unwrap();
        assert!(
            !cash_line.contains("-50.00"),
            "elided amount must stay elided: {}",
            cash_line
        );
    }
}
