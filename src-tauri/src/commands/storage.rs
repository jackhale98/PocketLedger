//! Mobile journal storage.
//!
//! On iOS the document picker hands the app a COPY of the picked file in the
//! temporary `tmp/<bundle-id>-Inbox` directory. iOS deletes that directory
//! between launches, so a persisted Inbox path never survives a relaunch and
//! any edits written to the copy are silently lost. The container UUID in
//! absolute paths also changes across app updates.
//!
//! The fix: on mobile, journals live in the app's own documents directory
//! (visible in the iOS Files app via UIFileSharingEnabled). Picked files are
//! imported (copied) there, and the frontend persists only the file NAME,
//! re-resolving it against the storage dir on every launch.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::journal::{normalize_path, JournalSummary};

/// Where journals live. iOS: the app's Documents dir (user-visible in the
/// Files app). Android: the private app data dir (persistent, backed up).
/// Desktop: an app-data subfolder — unused by the UI (desktop opens files in
/// place) but kept valid so the commands work everywhere.
pub fn storage_dir(app: &AppHandle) -> Result<PathBuf, String> {
    // iOS: $HOME/Documents inside the app container — the folder Files shows
    // under "On My iPhone > PocketHLedger", so a git client can link to it.
    if cfg!(target_os = "ios") {
        if let Ok(dir) = app.path().document_dir() {
            if fs::create_dir_all(&dir).is_ok() {
                ensure_visible_in_files(&dir);
                return Ok(dir);
            }
        }
        // Falling back keeps the app usable even though the folder is then
        // private to the app and not visible to Files or a git client.
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data directory: {e}"))?
        .join("journals");
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// iOS only lists an app's Documents folder in the Files app once it holds at
/// least one file — an empty folder simply doesn't appear, which looks like
/// the file-sharing keys are broken. Drop a README so it's always there.
/// The placeholder `ensure_visible_in_files` drops; never listed as a journal.
const README_NAME: &str = "README.txt";

fn ensure_visible_in_files(dir: &std::path::Path) {
    let Ok(mut entries) = fs::read_dir(dir) else {
        return;
    };
    if entries.next().is_some() {
        return;
    }
    let _ = fs::write(
        dir.join(README_NAME),
        "PocketHLedger keeps your journals in this folder.\n\n         Files you add here appear in the app. Journals that use `include`\n         need the included files here too, alongside the main journal.\n\n         To version these files with git, use an iOS git client that can work\n         on a folder in place and point it at this folder.\n",
    );
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformInfo {
    pub is_mobile: bool,
    pub storage_dir: String,
}

#[tauri::command]
pub async fn platform_info(app: AppHandle) -> Result<PlatformInfo, String> {
    Ok(PlatformInfo {
        is_mobile: cfg!(any(target_os = "ios", target_os = "android")),
        storage_dir: storage_dir(&app)?.to_string_lossy().into_owned(),
    })
}

/// Resolve a persisted journal reference to an absolute path. On mobile the
/// reference is relative to the storage dir; refs saved by older versions
/// were absolute and are migrated by falling back to the file name.
#[tauri::command]
pub async fn resolve_journal_ref(reference: String, app: AppHandle) -> Result<String, String> {
    if !cfg!(any(target_os = "ios", target_os = "android")) {
        return Ok(reference);
    }
    let dir = storage_dir(&app)?;
    let candidate = dir.join(&reference);
    if candidate.is_file() {
        return Ok(candidate.to_string_lossy().into_owned());
    }
    // Older builds persisted the full container path, whose UUID has since
    // changed; recover by name.
    let base = reference.rsplit('/').next().unwrap_or(&reference);
    let by_name = dir.join(base);
    if by_name.is_file() {
        return Ok(by_name.to_string_lossy().into_owned());
    }
    // Fall back to the raw ref so the caller reports a real "not found".
    Ok(candidate.to_string_lossy().into_owned())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredJournal {
    pub name: String,
    pub path: String,
    /// Unix seconds; 0 if unknown.
    pub modified: u64,
    pub size: u64,
}

const JOURNAL_EXTENSIONS: &[&str] = &["journal", "ledger", "hledger", "j", "txt", "dat"];

/// How deep to look for journals below the storage dir. Users who point a git
/// client (e.g. Working Copy's "Link Repository to Folder") at this directory
/// often organize journals into subfolders like `2024/`.
const MAX_SCAN_DEPTH: usize = 3;

/// Journals in the storage dir, newest first. `name` is the path RELATIVE to
/// the storage dir, which is what gets persisted as "last journal" — absolute
/// container paths change whenever iOS updates the app.
#[tauri::command]
pub async fn list_stored_journals(app: AppHandle) -> Result<Vec<StoredJournal>, String> {
    let dir = storage_dir(&app)?;
    let mut out = Vec::new();
    scan_dir(&dir, &dir, 0, &mut out)?;
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.name.cmp(&b.name)));
    Ok(out)
}

fn scan_dir(
    root: &std::path::Path,
    dir: &std::path::Path,
    depth: usize,
    out: &mut Vec<StoredJournal>,
) -> Result<(), String> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        // A single unreadable subfolder must not fail the whole listing.
        Err(_) if depth > 0 => return Ok(()),
        Err(e) => return Err(format!("Cannot read {}: {e}", dir.display())),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skips .git and friends — never list or touch a git client's data.
        if name.starts_with('.') {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            if depth + 1 < MAX_SCAN_DEPTH {
                scan_dir(root, &path, depth + 1, out)?;
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !JOURNAL_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        // Our own placeholder (README.txt) has a journal extension but isn't one.
        if depth == 0 && name == README_NAME {
            continue;
        }
        let meta = entry.metadata().ok();
        let modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let size = meta.map(|m| m.len()).unwrap_or(0);
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        out.push(StoredJournal {
            name: rel,
            path: path.to_string_lossy().into_owned(),
            modified,
            size,
        });
    }
    Ok(())
}

/// Remove a journal from app storage. `name` is the storage-relative name
/// from `list_stored_journals`; anything that escapes the storage directory
/// is refused rather than trusted.
#[tauri::command]
pub async fn delete_stored_journal(
    name: String,
    app: AppHandle,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<(), String> {
    let dir = storage_dir(&app)?;
    let target = dir.join(&name);

    // A traversal-safe check that doesn't need the file to still exist.
    let canonical_dir = fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
    let canonical_target = match fs::canonicalize(&target) {
        Ok(p) => p,
        Err(e) => return Err(format!("Cannot remove '{name}': {e}")),
    };
    if !canonical_target.starts_with(&canonical_dir) || !canonical_target.is_file() {
        return Err(format!("'{name}' is not a journal in this app's folder."));
    }

    fs::remove_file(&canonical_target).map_err(|e| format!("Cannot remove '{name}': {e}"))?;
    // Its backup is ours too, and leaving it behind would strand the data.
    // Backups live in the backup dir under a path-hashed name (see
    // journal::backup_path), keyed by the path the journal was loaded from.
    let backup_dir = crate::lock_or_recover(&state).backup_dir.clone();
    if let Some(bdir) = backup_dir {
        for candidate in [&target, &canonical_target] {
            let _ = fs::remove_file(super::journal::backup_path(&bdir, candidate));
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedJournal {
    pub path: String,
    pub file_name: String,
    /// The stored copy was byte-identical, so it was reused as-is.
    pub reused: bool,
    /// A different journal with that name already existed; imported under a
    /// numbered name instead (never overwrites in-app edits).
    pub renamed: bool,
}

/// True for paths the OS may delete out from under us: the iOS document
/// picker's `tmp/<bundle-id>-Inbox` copy and anything else under tmp/caches.
/// Loading a journal from one of these means later writes fail with
/// "can no longer be read" — and edits made in between are lost.
pub fn is_transient_path(path: &std::path::Path) -> bool {
    cfg!(any(target_os = "ios", target_os = "android"))
        && looks_transient(&path.to_string_lossy())
}

fn looks_transient(s: &str) -> bool {
    s.contains("-Inbox/") || s.contains("/tmp/") || s.contains("/Caches/")
}

/// If `path` points into a volatile OS directory, copy it into app storage
/// and return the durable path; otherwise return the path unchanged.
pub fn relocate_if_transient(path: &str, app: &AppHandle) -> Result<String, String> {
    let src = normalize_path(path);
    if !is_transient_path(&src) {
        return Ok(path.to_string());
    }
    Ok(import_into_storage(&src, app)?.path)
}

/// Copy a picked file into the storage dir. Never overwrites: an identical
/// existing copy is reused, a conflicting one causes a numbered rename.
#[tauri::command]
pub async fn import_journal_file(path: String, app: AppHandle) -> Result<ImportedJournal, String> {
    super::journal::reject_unsupported_uri(&path)?;
    import_into_storage(&normalize_path(&path), &app)
}

/// Store journal text the frontend already read itself. Android's document
/// picker yields `content://` URIs that `std::fs` can't open, but the fs
/// plugin's `readTextFile` can; the frontend reads the file and hands us the
/// text plus the display name, and it lands in storage exactly as a picked
/// file would. The text must be a parseable journal so a mis-picked file is
/// refused before it's stored.
#[tauri::command]
pub async fn import_journal_text(
    name: String,
    text: String,
    app: AppHandle,
) -> Result<ImportedJournal, String> {
    let file_name = sanitize_journal_name(&name)?;
    if text.trim().is_empty() {
        return Err(format!("'{file_name}' is empty — nothing to import."));
    }
    hledger_parser::parse(&text).map_err(|e| format!("'{file_name}' is not a journal: {e}"))?;
    store_bytes(None, &file_name, text.as_bytes(), &app)
}

fn import_into_storage(
    src: &std::path::Path,
    app: &AppHandle,
) -> Result<ImportedJournal, String> {
    let bytes =
        fs::read(src).map_err(|e| format!("Cannot read {}: {e}", src.display()))?;
    if std::str::from_utf8(&bytes).is_err() {
        return Err(format!(
            "'{}' is not a text file — journals must be plain text.",
            src.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| src.display().to_string())
        ));
    }

    let file_name = src
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("imported.journal")
        .to_string();
    store_bytes(Some(src), &file_name, &bytes, app)
}

/// The shared tail of importing: place `bytes` under `file_name` in storage.
/// `src` is the picked file when there is one (for the self-copy check and
/// Inbox cleanup); None when the bytes came from the frontend.
fn store_bytes(
    src: Option<&std::path::Path>,
    file_name: &str,
    bytes: &[u8],
    app: &AppHandle,
) -> Result<ImportedJournal, String> {
    let dir = storage_dir(app)?;
    let file_name = file_name.to_string();
    let cleanup = |src: Option<&std::path::Path>| {
        if let Some(src) = src {
            cleanup_inbox_copy(src);
        }
    };

    let dest = dir.join(&file_name);
    // Refuse to copy a file onto itself (already stored).
    if src == Some(dest.as_path()) {
        return Ok(ImportedJournal {
            path: dest.to_string_lossy().into_owned(),
            file_name,
            reused: true,
            renamed: false,
        });
    }

    if dest.exists() {
        match fs::read(&dest) {
            Ok(existing) if existing == bytes => {
                cleanup(src);
                return Ok(ImportedJournal {
                    path: dest.to_string_lossy().into_owned(),
                    file_name,
                    reused: true,
                    renamed: false,
                });
            }
            _ => {
                let (stem, ext) = split_name(&file_name);
                for n in 2..1000 {
                    let candidate_name = if ext.is_empty() {
                        format!("{stem}-{n}")
                    } else {
                        format!("{stem}-{n}.{ext}")
                    };
                    let candidate = dir.join(&candidate_name);
                    if !candidate.exists() {
                        fs::write(&candidate, bytes)
                            .map_err(|e| format!("Cannot write {}: {e}", candidate.display()))?;
                        cleanup(src);
                        return Ok(ImportedJournal {
                            path: candidate.to_string_lossy().into_owned(),
                            file_name: candidate_name,
                            reused: false,
                            renamed: true,
                        });
                    }
                }
                return Err("Too many journals with that name.".to_string());
            }
        }
    }

    fs::write(&dest, bytes).map_err(|e| format!("Cannot write {}: {e}", dest.display()))?;
    cleanup(src);
    Ok(ImportedJournal {
        path: dest.to_string_lossy().into_owned(),
        file_name,
        reused: false,
        renamed: false,
    })
}

/// The iOS picker's Inbox copy belongs to the app and iOS never cleans it up
/// promptly; delete it once we've secured our own copy. Only touches paths
/// inside the picker Inbox — never a real user file.
fn cleanup_inbox_copy(src: &std::path::Path) {
    if src.to_string_lossy().contains("-Inbox/") {
        let _ = fs::remove_file(src);
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedJournal {
    pub path: String,
    pub file_name: String,
    pub summary: JournalSummary,
}

/// Create a new journal by NAME inside the storage dir (mobile has no usable
/// save dialog — the frontend asks for a name instead).
#[tauri::command]
pub async fn create_stored_journal(
    name: String,
    default_currency: Option<String>,
    app: AppHandle,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<CreatedJournal, String> {
    let file_name = sanitize_journal_name(&name)?;
    let path = storage_dir(&app)?.join(&file_name);
    let path_str = path.to_string_lossy().into_owned();
    let summary =
        super::journal::create_journal(path_str.clone(), default_currency, state).await?;
    Ok(CreatedJournal {
        path: path_str,
        file_name,
        summary,
    })
}

/// Copy a just-picked file (CSV / rules) into the app cache so it outlives
/// the iOS picker Inbox, which can be cleaned at any time. Returns the
/// stable stashed path to use for preview and import.
#[tauri::command]
pub async fn stash_picked_file(path: String, app: AppHandle) -> Result<String, String> {
    let src = normalize_path(&path);
    let bytes =
        fs::read(&src).map_err(|e| format!("Cannot read {}: {e}", src.display()))?;

    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("Cannot resolve cache directory: {e}"))?
        .join("picked-files");
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let file_name = src
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("picked");
    let dest = dir.join(format!("{nonce}-{file_name}"));
    fs::write(&dest, &bytes).map_err(|e| format!("Cannot write {}: {e}", dest.display()))?;
    cleanup_inbox_copy(&src);
    Ok(dest.to_string_lossy().into_owned())
}

/// Turn a user-typed name into a safe file name inside the storage dir:
/// no separators (so it can't escape the directory), no leading dots (which
/// would make it hidden and thus invisible in the journal list), and a
/// journal extension.
fn sanitize_journal_name(name: &str) -> Result<String, String> {
    let cleaned: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '/' || c == '\\' || c == ':' { '-' } else { c })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').trim();
    if cleaned.is_empty() {
        return Err("Enter a name for the journal file.".to_string());
    }
    let (_, ext) = split_name(cleaned);
    Ok(
        if JOURNAL_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            cleaned.to_string()
        } else {
            format!("{cleaned}.journal")
        },
    )
}

fn split_name(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_string()),
        _ => (name.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::{looks_transient, sanitize_journal_name, scan_dir, split_name, README_NAME};

    #[test]
    fn readme_placeholder_is_not_listed_as_a_journal() {
        let dir = std::env::temp_dir().join(format!("pockethledger-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(README_NAME), "hello").unwrap();
        std::fs::write(dir.join("notes.txt"), "2024-01-01 T\n").unwrap();
        std::fs::write(dir.join("main.journal"), "").unwrap();
        let mut out = Vec::new();
        scan_dir(&dir, &dir, 0, &mut out).unwrap();
        let mut names: Vec<_> = out.iter().map(|j| j.name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["main.journal".to_string(), "notes.txt".to_string()]);
    }

    #[test]
    fn split_name_handles_extensions() {
        assert_eq!(split_name("a.journal"), ("a".into(), "journal".into()));
        assert_eq!(split_name("a.b.journal"), ("a.b".into(), "journal".into()));
        assert_eq!(split_name("noext"), ("noext".into(), String::new()));
        assert_eq!(split_name(".hidden"), (".hidden".into(), String::new()));
    }

    #[test]
    fn sanitize_adds_journal_extension_only_when_missing() {
        assert_eq!(sanitize_journal_name("finances").unwrap(), "finances.journal");
        assert_eq!(sanitize_journal_name("2024.journal").unwrap(), "2024.journal");
        assert_eq!(sanitize_journal_name("books.ledger").unwrap(), "books.ledger");
        // An unrelated extension is kept as part of the stem, not trusted.
        assert_eq!(sanitize_journal_name("notes.pdf").unwrap(), "notes.pdf.journal");
    }

    #[test]
    fn sanitize_strips_path_separators_so_names_cannot_escape_storage() {
        for input in ["../../etc/passwd", "/etc/passwd", "a/b", "a\\b", "a:b", "/"] {
            let name = sanitize_journal_name(input).unwrap();
            assert!(!name.contains('/'), "{input} -> {name}");
            assert!(!name.contains('\\'), "{input} -> {name}");
            assert!(!name.starts_with('.'), "{input} -> {name}");
        }
    }

    #[test]
    fn sanitize_rejects_names_that_reduce_to_nothing() {
        for input in ["", "   ", ".", "..", "...", "\n", "\u{7}"] {
            assert!(sanitize_journal_name(input).is_err(), "accepted {input:?}");
        }
    }

    #[test]
    fn transient_paths_are_recognized() {
        assert!(looks_transient(
            "/var/mobile/Containers/Data/Application/A21E/tmp/com.x.app-Inbox/2023.journal"
        ));
        assert!(looks_transient("/var/mobile/.../tmp/scratch.journal"));
        assert!(looks_transient("/var/mobile/Library/Caches/x.journal"));
        assert!(!looks_transient(
            "/var/mobile/Containers/Data/Application/A21E/Documents/2023.journal"
        ));
    }
}
