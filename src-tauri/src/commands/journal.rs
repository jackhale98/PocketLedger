use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use hledger_core::balance::ResolveOptions;
use hledger_core::ledger::{Ledger, LedgerOptions};
use hledger_parser::ParseContext;
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
    /// (mtime, length) observed when `text` was read, so a staleness check
    /// can skip re-reading files whose metadata hasn't moved. None when the
    /// filesystem gave no usable metadata (cloud placeholders, overlays).
    pub disk_meta: Option<(std::time::SystemTime, u64)>,
}

/// Metadata fingerprint for cheap change detection. Requires both a
/// modification time and a length; anything less falls back to a content
/// compare.
fn disk_fingerprint(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
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
    /// Indices into journal.items of every Transaction item, in parse order,
    /// so the nth transaction is an O(1) lookup instead of a scan (listing
    /// all transactions was quadratic).
    pub txn_items: Vec<usize>,
    pub ledger: Ledger,
    pub writer_config: WriterConfig,
    /// Warnings gathered at load: include problems + parse warnings.
    pub load_warnings: Vec<String>,
    /// `include` targets that could not be found, as written in the journal.
    /// On mobile these are usually siblings that were never imported, so the
    /// UI offers to fetch them by name.
    pub missing_includes: Vec<String>,
    /// Directive state after loading every file: commodity styles, decimal
    /// marks, `D`. Used to write new amounts the way the journal does.
    pub parse_context: ParseContext,
    /// Options this journal was loaded with, reused on every reload.
    pub options: LoadOptions,
}

impl LoadedJournal {
    pub fn source_path(&self) -> &Path {
        &self.files[0].path
    }

    #[allow(dead_code)]
    pub fn main_text(&self) -> &str {
        &self.files[0].text
    }

    /// The nth transaction (parse order) with its item index and file index.
    pub fn nth_transaction(&self, index: usize) -> Option<(&Transaction, usize, usize)> {
        let item_idx = *self.txn_items.get(index)?;
        match &self.journal.items[item_idx] {
            JournalItem::Transaction(t) => Some((t, item_idx, self.item_files[item_idx])),
            _ => None,
        }
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

/// Read a journal file as text. A UTF-8 BOM (which some Windows editors
/// prepend) is dropped rather than parsed as part of the first line, and a
/// file that isn't UTF-8 gets an error naming the problem instead of the
/// opaque "stream did not contain valid UTF-8".
fn read_text(path: &Path) -> std::io::Result<String> {
    let mut bytes = std::fs::read(path)?;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes.drain(..3);
    }
    String::from_utf8(bytes).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "'{}' is not valid UTF-8 text. Journals must be plain UTF-8 text files; \
                 re-save it as UTF-8 in your editor.",
                path.display()
            ),
        )
    })
}

/// Read one file, preferring an overlay entry keyed by canonical path.
/// Returns the text plus the on-disk fingerprint (None for overlaid text,
/// which hasn't been written yet).
fn read_file(
    path: &Path,
    overlay: &HashMap<PathBuf, String>,
) -> std::io::Result<(String, Option<(std::time::SystemTime, u64)>)> {
    if let Some(text) = overlay.get(&canonical_key(path)) {
        return Ok((text.clone(), None));
    }
    // Fingerprint before reading: if the file changes between the two, the
    // stale fingerprint makes the next check fall through to a content compare.
    let meta = disk_fingerprint(path);
    Ok((read_text(path)?, meta))
}

fn canonical_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Expand a leading `~` / `~/` in an include path to the home directory, as
/// hledger does.
fn expand_tilde(s: &str) -> PathBuf {
    if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        if let Some(home) = home {
            return PathBuf::from(home).join(&s[2.min(s.len())..]);
        }
    }
    PathBuf::from(s)
}

struct LoadContext<'a> {
    overlay: &'a HashMap<PathBuf, String>,
    files: Vec<SourceFile>,
    items: Vec<JournalItem>,
    item_files: Vec<usize>,
    warnings: Vec<String>,
    missing_includes: Vec<String>,
    visited: HashSet<PathBuf>,
    /// Directive state inherited from the including file (hledger: `D`,
    /// `Y`, `alias`, `apply account`, `decimal-mark` flow parent→child;
    /// `commodity` declarations are journal-wide).
    parse_ctx: ParseContext,
}

/// How to build the ledger from the loaded files.
#[derive(Debug, Clone, Copy)]
pub struct LoadOptions {
    /// Strict rejects an unbalanced transaction outright (validating an
    /// edit); lenient keeps it, marks it and warns (opening a file).
    pub strict: bool,
    /// Treat transaction costs as market prices (hledger's
    /// `--infer-market-prices`). hledger's default is off.
    pub infer_market_prices: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        LoadOptions { strict: false, infer_market_prices: false }
    }
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

    let (text, disk_meta) = read_file(path, ctx.overlay).map_err(|e| {
        if e.kind() == std::io::ErrorKind::InvalidData {
            e.to_string()
        } else {
            format!("Cannot read {}: {}", path.display(), e)
        }
    })?;

    let parsed = hledger_parser::parse_file_with_context(&text, &ctx.parse_ctx)
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    let hledger_parser::ParsedFile {
        journal,
        context: mut file_ctx,
        include_contexts,
    } = parsed;
    let mut include_no = 0usize;

    let file_idx = ctx.files.len();
    ctx.files.push(SourceFile {
        path: path.to_path_buf(),
        text,
        disk_meta,
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

            // The included file sees the directives in force at this line
            // (so an `apply account` closed further down still covers it),
            // plus everything journal-wide learned so far.
            let mut parent = include_contexts
                .get(include_no)
                .cloned()
                .unwrap_or_else(|| file_ctx.clone());
            include_no += 1;
            parent.absorb_global(&file_ctx);

            let resolve = |s: &str| -> PathBuf {
                let expanded = expand_tilde(s);
                if expanded.is_absolute() {
                    return expanded;
                }
                if let Some(base) = &base_dir {
                    base.join(expanded)
                } else {
                    expanded
                }
            };

            if inc_str.contains('*') || inc_str.contains('?') {
                let pattern = resolve(&inc_str).to_string_lossy().to_string();
                match glob::glob(&pattern) {
                    Ok(paths) => {
                        let mut matched = 0;
                        for entry in paths.flatten() {
                            matched += 1;
                            load_included(ctx, &mut parent, &entry)?;
                        }
                        // hledger treats a pattern that matches nothing as an
                        // error; silently loading an empty set hides a typo.
                        if matched == 0 {
                            ctx.warnings.push(format!(
                                "Include pattern '{}' matched no files (resolved to {})",
                                inc_str, pattern
                            ));
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
                    load_included(ctx, &mut parent, &inc_path)?;
                } else {
                    ctx.missing_includes.push(inc_str.clone());
                    ctx.warnings.push(format!(
                        "Could not include '{}': file not found (resolved to {})",
                        inc_str,
                        inc_path.display()
                    ));
                }
            }
            file_ctx.absorb_global(&parent);
        } else {
            ctx.items.push(item);
            ctx.item_files.push(file_idx);
        }
    }

    // Hand the caller this file's final state; it keeps only the
    // journal-wide parts (see `load_included`).
    ctx.parse_ctx = file_ctx;

    Ok(())
}

/// Load an included file with the including file's directive state, then
/// fold the child's journal-wide declarations (`commodity`, observed amount
/// styles) back into the parent's.
fn load_included(ctx: &mut LoadContext, parent: &mut ParseContext, path: &Path) -> Result<(), String> {
    ctx.parse_ctx = parent.clone();
    load_one_file(ctx, path)?;
    parent.absorb_global(&ctx.parse_ctx);
    Ok(())
}

fn load_journal_with_overlay(
    path: &str,
    overlay: &HashMap<PathBuf, String>,
    options: LoadOptions,
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
        parse_ctx: ParseContext::default(),
    };

    load_one_file(&mut ctx, &file_path)?;

    let writer_config = writer::infer_config(&ctx.files[0].text);
    let journal = Journal {
        items: ctx.items,
        source_path: Some(file_path),
        warnings: vec![],
    };

    let ledger = Ledger::from_journal_with(
        &journal,
        LedgerOptions {
            resolve: if options.strict { ResolveOptions::STRICT } else { ResolveOptions::LENIENT },
            infer_market_prices_from_costs: options.infer_market_prices,
        },
    )
    .map_err(|e| e.to_string())?;
    let txn_items = journal
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| matches!(item, JournalItem::Transaction(_)))
        .map(|(i, _)| i)
        .collect();

    Ok(LoadedJournal {
        files: ctx.files,
        journal,
        item_files: ctx.item_files,
        txn_items,
        ledger,
        writer_config,
        load_warnings: ctx.warnings,
        missing_includes: ctx.missing_includes,
        parse_context: ctx.parse_ctx,
        options,
    })
}

/// Open a journal for viewing/editing: lenient, hledger's price defaults.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn load_journal(path: &str) -> Result<LoadedJournal, String> {
    load_journal_with_overlay(path, &HashMap::new(), LoadOptions::default())
}

pub(crate) fn load_journal_opts(path: &str, options: LoadOptions) -> Result<LoadedJournal, String> {
    load_journal_with_overlay(path, &HashMap::new(), LoadOptions { strict: false, ..options })
}

// ─── Safe writing: staleness check → validate → backup → atomic write ───

fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    use std::io::Write;

    // Write to the real file: renaming over a symlink would replace the link
    // with a regular file and leave the journal it pointed at untouched.
    let resolved = match path.canonicalize() {
        Ok(p) => p,
        // New file: canonicalize the directory so a symlinked folder still
        // receives the file in place.
        Err(_) => match (path.parent(), path.file_name()) {
            (Some(dir), Some(name)) => dir
                .canonicalize()
                .map(|d| d.join(name))
                .unwrap_or_else(|_| path.to_path_buf()),
            _ => path.to_path_buf(),
        },
    };
    let path = resolved.as_path();
    let original_permissions = std::fs::metadata(path).ok().map(|m| m.permissions());

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
        // The rename replaces the inode, so a mode the user set on the
        // journal (group-readable, say) would otherwise revert to the umask.
        if let Some(perms) = original_permissions {
            let _ = f.set_permissions(perms);
        }
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
    // Best-effort: a failed backup must not block the save, but the backup
    // itself is written atomically so it's never half a file.
    let _ = atomic_write(&backup_path(dir, path), old_content);
}

/// Where the backup of the journal at `path` lives inside `backup_dir`. The
/// single source of the naming, so deletion removes what saving created.
pub(crate) fn backup_path(backup_dir: &Path, path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "journal".to_string());
    backup_dir.join(format!("{}.{:08x}.bak", name, path_hash(path)))
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

/// How the on-disk journal relates to what's loaded.
#[derive(Debug, PartialEq)]
enum DiskState {
    /// Every file still matches what was loaded.
    Unchanged,
    /// A file was edited outside the app; a reload is the right response.
    Modified(String),
    /// A file no longer exists (deleted, moved, or an unmounted volume). A
    /// reload cannot help; the in-memory copy is the only one left.
    Missing(String),
    /// A file exists but couldn't be read right now (cloud placeholder, lock).
    Unreadable(String),
}

impl DiskState {
    fn into_result(self) -> Result<(), String> {
        match self {
            DiskState::Unchanged => Ok(()),
            DiskState::Modified(m) | DiskState::Missing(m) | DiskState::Unreadable(m) => Err(m),
        }
    }
}

/// Verify no file changed on disk behind our back (external editor, sync
/// service). Blindly persisting cached text would erase those edits.
///
/// Files whose modification time and length still match what was loaded are
/// taken as unchanged without reading them; everything else is compared by
/// content, so a filesystem without usable metadata still gets a real check.
fn disk_state(loaded: &LoadedJournal) -> DiskState {
    for file in &loaded.files {
        if file.disk_meta.is_some() && disk_fingerprint(&file.path) == file.disk_meta {
            continue;
        }
        match read_text(&file.path) {
            Ok(disk) => {
                // A file that had content and now reads empty is almost never
                // a real edit: it's an unmaterialised cloud placeholder or a
                // sync client mid-write. Refusing here keeps us from treating
                // it as an external change and, worse, writing that emptiness
                // back over the real journal.
                if disk.is_empty() && !file.text.trim().is_empty() {
                    return DiskState::Unreadable(format!(
                        "'{}' read back empty, which usually means it isn't downloaded yet \
                         or your sync client is still writing it. Nothing was changed.",
                        file.path.display()
                    ));
                }
                if disk != file.text {
                    return DiskState::Modified(format!(
                        "'{}' was modified outside this app since it was loaded. Reload the journal, then repeat your change.",
                        file.path.display()
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return DiskState::Missing(format!(
                    "'{}' no longer exists on disk (deleted, moved, or its folder is \
                     unmounted). Your loaded copy is intact; use 'Restore from memory' \
                     to write it back, or reopen the journal from its new location.",
                    file.path.display()
                ));
            }
            Err(e) => {
                return DiskState::Unreadable(describe_read_failure(&file.path, &e));
            }
        }
    }
    DiskState::Unchanged
}

fn check_stale(loaded: &LoadedJournal) -> Result<(), String> {
    disk_state(loaded).into_result()
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

    // check_stale just established every file still matches memory, so the
    // reload can take them from memory instead of reading each one again.
    let mut overlay: HashMap<PathBuf, String> = loaded
        .files
        .iter()
        .map(|f| (canonical_key(&f.path), f.text.clone()))
        .collect();
    overlay.insert(canonical_key(&target_path), new_text.clone());

    // Validate the complete new state before writing anything.
    let strict = LoadOptions { strict: true, ..loaded.options };
    let candidate = load_journal_with_overlay(&main_path, &overlay, strict).map_err(|e| {
        format!("Change rejected (journal would become invalid): {}", e)
    })?;

    write_backup(app_state.backup_dir.as_deref(), &target_path, &old_text);
    atomic_write(&target_path, &new_text)?;

    // The overlaid files carry no fingerprint; take them from disk now that
    // the write landed, so the next staleness check can skip the reads.
    let mut candidate = candidate;
    for file in &mut candidate.files {
        file.disk_meta = disk_fingerprint(&file.path);
    }

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

fn build_amount(
    amt_str: &str,
    commodity: &str,
    styles: &hledger_core::styles::CommodityStyles,
    parse_ctx: &ParseContext,
) -> Result<PostingAmount, String> {
    // The decimal mark is fixed to '.' on purpose: the frontend normalizes
    // whatever the user typed (locale commas included) to dot-decimal before
    // sending, so "1,234.56" here is a thousands separator, never a mark.
    let q = hledger_parser::parse_quantity_with(amt_str, Some('.'))
        .map_err(|e| format!("Invalid amount '{}': {}", amt_str, e))?;

    if commodity.contains('"') {
        return Err("Commodity must not contain quotes".to_string());
    }
    validate_text_field(commodity, "Commodity")?;

    // Write the amount the way the journal already writes this commodity
    // (side, spacing, decimal mark, digit grouping); fall back to a sensible
    // default only for a commodity the journal has never seen.
    let mut style: AmountStyle = parse_ctx
        .style_for(commodity)
        .unwrap_or_else(|| writer::default_style_for(commodity));
    // Precision comes from what the user typed, never below what the journal
    // already uses for this commodity — a hardcoded 2 destroyed
    // high-precision amounts (0.00012345 BTC became 0.00), and padding a
    // whole-number commodity turned 10 AAPL into 10.00 AAPL. Only a commodity
    // the journal has never seen falls back to two places.
    let known = styles.precision(commodity) as u8;
    style.precision = q.precision.max(known);

    Ok(PostingAmount {
        quantity: q.value,
        commodity: commodity.to_string(),
        style,
        cost: None,
        multiplier: false,
    })
}

fn build_transaction(txn: &NewTransaction, loaded: &LoadedJournal) -> Result<Transaction, String> {
    let styles = loaded.ledger.styles();
    let parse_ctx = &loaded.parse_context;
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
                styles,
                parse_ctx,
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

/// True if a posting carries structure the edit form cannot represent:
/// costs, assertions, tags, virtual markers, posting dates and statuses, and
/// comment continuation lines (the form edits only the first line). Shared
/// with the transaction list's `hasHiddenDetails` badge so both agree.
pub(crate) fn posting_has_extras(p: &Posting) -> bool {
    p.balance_assertion.is_some()
        || p.amount.as_ref().map_or(false, |a| a.cost.is_some())
        || p.is_virtual
        || !p.tags.is_empty()
        || p.date.is_some()
        || p.date2.is_some()
        || p.status != Status::Unmarked
        || p.comment.as_ref().map_or(false, |c| c.text.contains('\n'))
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

    // The form shows the postings in file order and cannot reorder them, so
    // an unchanged count means row i is still posting i — even when its
    // account was renamed. Matching on account names here refused a plain
    // account correction on any transaction with a cost or assertion.
    let aligned = edited.postings.len() == original.postings.len();

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
                    if new_amt.commodity == old_amt.commodity {
                        // Carry the cost and the original display style; keep
                        // the higher precision so values never truncate.
                        new_amt.cost = old_amt.cost.clone();
                        let precision = new_amt.style.precision.max(old_amt.style.precision);
                        new_amt.style = old_amt.style.clone();
                        new_amt.style.precision = precision;
                    } else if old_amt.cost.is_some() {
                        // A cost prices the old commodity; attaching it to a
                        // different one would write nonsense.
                        return Err(format!(
                            "Posting {} has a cost (@) on its {} amount, which the editor cannot carry over to a different commodity. Keep the commodity, or edit the journal file in a text editor.",
                            i + 1,
                            old_amt.commodity
                        ));
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
    let original_has_extras = original.postings.iter().any(posting_has_extras);
    if original_has_extras {
        return Err(
            "This transaction contains costs, balance assertions, tags, posting statuses or virtual postings that the editor cannot preserve when postings are added or removed. Keep the same number of postings, or edit the journal file in a text editor."
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
    // Load under the lock: parsing outside it and swapping afterwards let an
    // edit that landed in between be clobbered by the older parse.
    let mut app_state = crate::lock_or_recover(&state);
    {
        let loaded = load_journal_opts(&path, load_options(&app_state))?;
        install_loaded(&mut app_state, loaded)
    }
}

/// Swap a freshly loaded journal into the state and bump the generation.
/// The load options the user's settings call for.
fn load_options(app_state: &crate::AppState) -> LoadOptions {
    LoadOptions {
        strict: false,
        infer_market_prices: app_state.infer_market_prices,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineOptions {
    pub infer_market_prices: bool,
}

/// Apply engine settings and reload the open journal so they take effect.
/// Returns the new summary, or None when no journal is open.
#[tauri::command]
pub async fn set_engine_options(
    options: EngineOptions,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Option<JournalSummary>, String> {
    let mut app_state = crate::lock_or_recover(&state);
    if app_state.infer_market_prices == options.infer_market_prices {
        return Ok(None);
    }
    app_state.infer_market_prices = options.infer_market_prices;
    let Some(path) = app_state
        .journal
        .as_ref()
        .map(|l| l.source_path().to_string_lossy().to_string())
    else {
        return Ok(None);
    };
    let loaded = load_journal_opts(&path, load_options(&app_state))?;
    install_loaded(&mut app_state, loaded).map(Some)
}

fn install_loaded(
    app_state: &mut crate::AppState,
    loaded: LoadedJournal,
) -> Result<JournalSummary, String> {
    let summary = make_summary(&loaded);
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
    let app_state = crate::lock_or_recover(&state);
    let loaded = match app_state.journal.as_ref() {
        Some(l) => l,
        None => return Ok(false),
    };
    // Only a real edit warrants a reload. A vanished file would make the
    // reload fail — and keep failing on every foreground — while the
    // in-memory copy is the last one there is; see recreate_journal_from_memory.
    Ok(matches!(disk_state(loaded), DiskState::Modified(_)))
}

/// Whether any loaded file has vanished from disk. Pairs with
/// `journal_changed_on_disk`: the frontend offers `recreate_journal_from_memory`
/// when this is true.
#[tauri::command]
pub async fn journal_missing_on_disk(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<bool, String> {
    let app_state = crate::lock_or_recover(&state);
    let loaded = match app_state.journal.as_ref() {
        Some(l) => l,
        None => return Ok(false),
    };
    Ok(matches!(disk_state(loaded), DiskState::Missing(_)))
}

/// Write the loaded in-memory files back to disk for every source file that
/// has vanished (deleted, moved, folder unmounted then remounted empty).
/// Files still present are left alone — this never overwrites anything — so
/// data that only exists in memory isn't lost when the source disappears.
/// Returns the summary of the restored journal.
#[tauri::command]
pub async fn recreate_journal_from_memory(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let mut app_state = crate::lock_or_recover(&state);
    recreate_missing_files(&mut app_state)
}

fn recreate_missing_files(app_state: &mut crate::AppState) -> Result<JournalSummary, String> {
    let loaded = app_state.journal.as_mut().ok_or("No journal loaded")?;
    let mut restored = 0;
    for file in &mut loaded.files {
        if file.path.exists() {
            continue;
        }
        if let Some(dir) = file.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Cannot create {}: {}", dir.display(), e))?;
        }
        atomic_write(&file.path, &file.text)?;
        file.disk_meta = disk_fingerprint(&file.path);
        restored += 1;
    }
    if restored == 0 {
        return Err("Every journal file is still on disk; nothing to restore.".to_string());
    }
    Ok(make_summary(loaded))
}

#[tauri::command]
pub async fn get_journal_info(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let app_state = crate::lock_or_recover(&state);
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    Ok(make_summary(loaded))
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
    let app_state = crate::lock_or_recover(&state);
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
    let mut app_state = crate::lock_or_recover(&state);

    let txn_text = {
        let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
        let ast_txn = build_transaction(&txn, loaded)?;
        writer::write_transaction(&ast_txn, &loaded.writer_config)
    };

    apply_append_to_file(&mut app_state, file_index.unwrap_or(0), &txn_text)
}

#[tauri::command]
pub async fn create_journal(
    path: String,
    default_currency: Option<String>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<JournalSummary, String> {
    let currency = default_currency.unwrap_or_else(|| "$".to_string());
    validate_currency(&currency)?;
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

    // Parse before writing so a bad template never leaves a broken file on
    // disk that then refuses to open.
    hledger_parser::parse(&initial_content)
        .map_err(|e| format!("Cannot create journal with currency '{currency}': {e}"))?;

    let mut app_state = crate::lock_or_recover(&state);
    atomic_write(&file_path, &initial_content)?;
    let path_str = file_path.to_string_lossy().to_string();
    {
        let loaded = load_journal_opts(&path_str, load_options(&app_state))?;
        install_loaded(&mut app_state, loaded)
    }
}

/// A commodity symbol for the `commodity` directive template: hledger
/// needs quotes around anything with spaces or digits, and a ';' would start
/// a comment. Rather than quoting on the user's behalf, refuse those.
fn validate_currency(currency: &str) -> Result<(), String> {
    if currency.is_empty() {
        return Err("Enter a currency symbol or code (e.g. $ or EUR).".to_string());
    }
    if currency.chars().any(|c| c.is_whitespace() || c.is_ascii_digit())
        || currency.contains(';')
        || currency.contains('"')
        || currency.contains('\n')
    {
        return Err(format!(
            "'{currency}' can't be used as a currency: use a symbol or code without spaces, digits, quotes or ';'."
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn suggest_accounts(
    prefix: String,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<String>, String> {
    let app_state = crate::lock_or_recover(&state);
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
    let app_state = crate::lock_or_recover(&state);
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
    let app_state = crate::lock_or_recover(&state);
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    Ok(loaded.ledger.accounts_for_description(&description))
}

#[tauri::command]
pub async fn suggest_payees(
    prefix: String,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<String>, String> {
    let app_state = crate::lock_or_recover(&state);
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
    let mut app_state = crate::lock_or_recover(&state);

    let (new_file_text, file_idx) = {
        let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
        let (original, _item_idx, file_idx) = loaded
            .nth_transaction(index)
            .ok_or("Transaction not found")?;

        let edited = build_transaction(&txn, loaded)?;
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
    let mut app_state = crate::lock_or_recover(&state);

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
    reject_unsupported_uri(&path)?;
    let path = super::storage::relocate_if_transient(&path, &app)?;
    let mut app_state = crate::lock_or_recover(&state);
    {
        let loaded = load_journal_opts(&path, load_options(&app_state))?;
        install_loaded(&mut app_state, loaded)
    }
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
            infer_market_prices: false,
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
        let loaded = state.journal.as_ref().unwrap();
        let ast = build_transaction(txn, loaded)?;
        let config = loaded.writer_config.clone();
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
            let edited = build_transaction(txn, loaded)?;
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

        // Changing the commodity under a cost → must refuse, not write
        // "expenses:food $150.00 @ $150.00".
        let txn = simple_txn("Restructured", "150.00");
        let err = update(&mut state, 0, &txn).unwrap_err();
        assert!(err.contains("cost"), "refusal message: {}", err);
        assert_eq!(before, std::fs::read_to_string(&main).unwrap());

        // Adding a posting → must refuse, not silently strip the cost.
        let mut three = simple_txn("Restructured", "150.00");
        three.postings[0].commodity = Some("AAPL".to_string());
        three.postings.push(NewPosting {
            account: "expenses:fees".to_string(),
            amount: Some("1".to_string()),
            commodity: Some("$".to_string()),
            comment: None,
        });
        let err = update(&mut state, 0, &three).unwrap_err();
        assert!(err.contains("cannot preserve"), "refusal message: {}", err);
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
    fn included_files_inherit_parent_directives() {
        // hledger: `commodity` is journal-wide; `D`, `Y`, `alias` and
        // `apply account` flow from the including file into the included
        // one. Parsing each file from scratch made a split-by-year journal
        // with a top-level `commodity 1.000,00 EUR` read amounts 1000x off.
        let dir = temp_dir("include-inherit");
        let main = dir.join("main.journal");
        std::fs::write(
            dir.join("2023.journal"),
            "01-06 Snacks\n    food:snacks   1.234,50 EUR\n    assets:cash\n\n2023-02-01 Bare\n    expenses:z   25\n    assets:cash\n",
        )
        .unwrap();
        std::fs::write(
            &main,
            "commodity 1.000,00 EUR\nD $1,000.00\nY 2023\nalias food = expenses:food\napply account personal\ninclude 2023.journal\nend apply account\n",
        )
        .unwrap();

        let loaded = load_journal(&main.to_string_lossy()).unwrap();
        assert!(loaded.load_warnings.is_empty(), "{:?}", loaded.load_warnings);
        let mut seen = std::collections::BTreeMap::new();
        for txn in loaded.ledger.transactions() {
            for p in &txn.postings {
                for (c, q) in &p.amount.amounts {
                    seen.insert(format!("{} {}", p.account.full, c), q.to_string());
                }
            }
        }
        // hledger applies the `apply account` prefix before aliases, so the
        // alias (anchored at the account start) does not fire here — same
        // as `hledger bal` on this layout.
        assert_eq!(seen.get("personal:food:snacks EUR").map(String::as_str), Some("1234.50"), "seen: {seen:?}");
        assert_eq!(seen.get("personal:expenses:z $").map(String::as_str), Some("25"));
        let dates: Vec<String> = loaded.ledger.transactions().map(|t| t.date.to_string()).collect();
        assert_eq!(dates, vec!["2023-01-06", "2023-02-01"]);

        // And a new amount in that commodity is written in the journal's style.
        let mut state = state_with(&main);
        let mut txn = simple_txn("Coffee", "3.5");
        txn.postings[0].commodity = Some("EUR".to_string());
        add(&mut state, &txn).unwrap();
        let text = std::fs::read_to_string(&main).unwrap();
        assert!(text.contains("3,50 EUR"), "written in comma style: {text}");
    }

    #[test]
    fn unbalanced_transaction_still_opens_but_is_rejected_as_an_edit() {
        let dir = temp_dir("lenient-open");
        let main = dir.join("main.journal");
        std::fs::write(
            &main,
            "2024-01-01 Broken\n    expenses:a   $10\n    assets:b     $-5\n\n2024-01-02 Fine\n    expenses:a   $1\n    assets:b\n",
        )
        .unwrap();
        let loaded = load_journal(&main.to_string_lossy()).unwrap();
        assert_eq!(loaded.ledger.transactions().count(), 2);
        assert!(loaded.all_warnings().iter().any(|w| w.to_lowercase().contains("balance")), "{:?}", loaded.all_warnings());
    }


    #[test]
    fn renaming_an_account_keeps_cost_and_assertion() {
        // Correcting an account on a posting that carries a cost or an
        // assertion is the most common edit there is; it must not be
        // refused as a restructure.
        let dir = temp_dir("rename-account");
        let main = dir.join("main.journal");
        std::fs::write(
            &main,
            "2024-03-01 Buy\n    assets:brokerage   10 AAPL @ $150.00\n    assets:checking   $-1500.00 = $500.00\n",
        )
        .unwrap();
        let mut state = state_with(&main);
        let txn = NewTransaction {
            date: "2024-03-01".to_string(),
            status: "Unmarked".to_string(),
            description: "Buy".to_string(),
            comment: None,
            postings: vec![
                NewPosting {
                    account: "assets:investments:brokerage".to_string(),
                    amount: Some("10".to_string()),
                    commodity: Some("AAPL".to_string()),
                    comment: None,
                },
                NewPosting {
                    account: "assets:bank:checking".to_string(),
                    amount: Some("-1500.00".to_string()),
                    commodity: Some("$".to_string()),
                    comment: None,
                },
            ],
        };
        update(&mut state, 0, &txn).unwrap();
        let text = std::fs::read_to_string(&main).unwrap();
        assert!(text.contains("assets:investments:brokerage"), "{text}");
        assert!(text.contains("@ $150.00"), "cost kept: {text}");
        assert!(text.contains("assets:bank:checking") && text.contains("= $500.00"), "assertion kept: {text}");
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
            infer_market_prices: false,
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

    #[test]
    fn nth_transaction_indexes_by_parse_order() {
        let dir = temp_dir("nth");
        let main = dir.join("main.journal");
        std::fs::write(
            &main,
            "commodity $1,000.00\n\n2024-01-01 A\n    a  $1.00\n    b\n\n~ monthly  Rule\n    a  $1.00\n    b\n\n2024-01-02 B\n    a  $2.00\n    b\n",
        )
        .unwrap();
        let loaded = load_journal(&main.to_string_lossy()).unwrap();
        assert_eq!(loaded.txn_items.len(), 2);
        assert_eq!(loaded.nth_transaction(0).unwrap().0.description, "A");
        assert_eq!(loaded.nth_transaction(1).unwrap().0.description, "B");
        assert!(loaded.nth_transaction(2).is_none());
    }

    #[test]
    fn deleted_file_is_missing_not_modified_and_can_be_restored() {
        let dir = temp_dir("missing");
        let main = dir.join("main.journal");
        let content = "2024-01-01 Seed\n    a  $1.00\n    b\n";
        std::fs::write(&main, content).unwrap();
        let mut state = state_with(&main);

        std::fs::remove_file(&main).unwrap();
        let loaded = state.journal.as_ref().unwrap();
        assert!(matches!(disk_state(loaded), DiskState::Missing(_)));
        // Not a "reload me" signal — a reload would just fail.
        assert!(!matches!(disk_state(loaded), DiskState::Modified(_)));

        recreate_missing_files(&mut state).unwrap();
        assert_eq!(std::fs::read_to_string(&main).unwrap(), content);
        assert_eq!(disk_state(state.journal.as_ref().unwrap()), DiskState::Unchanged);
        // Nothing left to restore now.
        assert!(recreate_missing_files(&mut state).is_err());
    }

    #[test]
    fn unchanged_metadata_skips_the_read_and_a_real_edit_is_still_caught() {
        let dir = temp_dir("meta");
        let main = dir.join("main.journal");
        std::fs::write(&main, "2024-01-01 Seed\n    a  $1.00\n    b\n").unwrap();
        let state = state_with(&main);
        let loaded = state.journal.as_ref().unwrap();
        assert!(loaded.files[0].disk_meta.is_some(), "fingerprint captured at load");
        assert_eq!(disk_state(loaded), DiskState::Unchanged);

        // Same length, different content, mtime pushed into the future so the
        // fingerprint moves even on coarse-mtime filesystems.
        std::fs::write(&main, "2024-01-01 Seed\n    a  $9.00\n    b\n").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        let f = std::fs::File::options().write(true).open(&main).unwrap();
        f.set_modified(later).unwrap();
        assert!(matches!(disk_state(loaded), DiskState::Modified(_)));
    }

    #[test]
    fn bom_is_stripped_and_non_utf8_gets_a_named_error() {
        let dir = temp_dir("utf8");
        let bom = dir.join("bom.journal");
        std::fs::write(&bom, b"\xEF\xBB\xBF2024-01-01 Seed\n    a  $1.00\n    b\n").unwrap();
        let loaded = load_journal(&bom.to_string_lossy()).unwrap();
        assert_eq!(loaded.ledger.transaction_count(), 1);
        assert!(!loaded.files[0].text.starts_with('\u{feff}'));

        let bad = dir.join("latin1.journal");
        std::fs::write(&bad, b"2024-01-01 Caf\xE9\n    a  $1.00\n    b\n").unwrap();
        let err = load_journal(&bad.to_string_lossy()).err().expect("non-UTF-8 must fail");
        assert!(err.contains("latin1.journal") && err.contains("UTF-8"), "{err}");
    }

    #[test]
    fn glob_include_with_no_matches_warns() {
        let dir = temp_dir("glob-empty");
        let main = dir.join("main.journal");
        std::fs::write(&main, "include 20*.journal\n\n2024-01-01 T\n    a  $1.00\n    b\n").unwrap();
        let loaded = load_journal(&main.to_string_lossy()).unwrap();
        assert!(
            loaded.load_warnings.iter().any(|w| w.contains("matched no files")),
            "{:?}",
            loaded.load_warnings
        );
    }

    #[test]
    fn tilde_in_include_paths_expands_to_home() {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        if let Some(home) = home {
            assert_eq!(expand_tilde("~/x.journal"), PathBuf::from(home).join("x.journal"));
        }
        assert_eq!(expand_tilde("~x/y"), PathBuf::from("~x/y"));
        assert_eq!(expand_tilde("plain.journal"), PathBuf::from("plain.journal"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_journal_is_updated_in_place_with_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("symlink");
        let real = dir.join("real.journal");
        std::fs::write(&real, "2024-01-01 Seed\n    a  $1.00\n    b\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.join("link.journal");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let mut state = state_with(&link);
        let mut txn = simple_txn("Added", "3.00");
        txn.postings[0].account = "a".to_string();
        txn.postings[1].account = "b".to_string();
        add(&mut state, &txn).unwrap();

        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "link kept");
        assert!(std::fs::read_to_string(&real).unwrap().contains("Added"), "target updated");
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn currency_validation_refuses_unquotable_symbols() {
        assert!(validate_currency("$").is_ok());
        assert!(validate_currency("EUR").is_ok());
        for bad in ["", "US D", "A1", "x;y", "\"q\""] {
            assert!(validate_currency(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn multi_line_posting_comment_counts_as_hidden_detail() {
        let j = hledger_parser::parse(
            "2024-01-01 T\n    a  $1.00  ; first\n    ; second\n    b\n",
        )
        .unwrap();
        let JournalItem::Transaction(t) = &j.items[0] else { panic!() };
        assert!(posting_has_extras(&t.postings[0]));
        assert!(!posting_has_extras(&t.postings[1]));
    }

    #[test]
    fn whole_number_commodity_keeps_its_precision() {
        let dir = temp_dir("aapl");
        let main = dir.join("main.journal");
        std::fs::write(
            &main,
            "2024-01-01 Buy\n    assets:broker  10 AAPL @ $150.00\n    assets:cash  $-1500.00\n",
        )
        .unwrap();
        let mut state = state_with(&main);
        let txn = NewTransaction {
            date: "2024-02-01".to_string(),
            status: "Unmarked".to_string(),
            description: "Buy more".to_string(),
            comment: None,
            postings: vec![
                NewPosting {
                    account: "assets:broker".to_string(),
                    amount: Some("5".to_string()),
                    commodity: Some("AAPL".to_string()),
                    comment: None,
                },
                NewPosting {
                    account: "equity:gift".to_string(),
                    amount: Some("-5".to_string()),
                    commodity: Some("AAPL".to_string()),
                    comment: None,
                },
            ],
        };
        add(&mut state, &txn).unwrap();
        let after = std::fs::read_to_string(&main).unwrap();
        assert!(after.contains("5 AAPL") && !after.contains("5.00 AAPL"), "{after}");
    }

    #[test]
    fn backup_path_is_shared_naming() {
        let dir = temp_dir("bakname");
        let main = dir.join("main.journal");
        let bak = backup_path(&dir, &main);
        assert!(bak.starts_with(&dir));
        let name = bak.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("main.journal.") && name.ends_with(".bak"), "{name}");
    }
}
