import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import * as api from "../../api/commands";
import type { StoredJournal } from "../../api/types";
import { useJournalStore } from "../../store/journalStore";
import { useSettingsStore } from "../../store/settingsStore";
import { toPersistedJournalRef } from "../../utils/platform";

/** Mobile journal chooser. Journals live in the app's own documents folder
 *  (visible in the iOS Files app); external files picked with the document
 *  picker are COPIED in, because iOS only hands us a temp Inbox copy that is
 *  deleted between launches. */
export function MobileJournalPicker({
  mode,
  showCreate = false,
  onOpened,
}: {
  /** "open" for first load, "switch" when a journal is already loaded */
  mode: "open" | "switch";
  showCreate?: boolean;
  onOpened?: () => void;
}) {
  const { openJournal, switchJournal, error, clearError } = useJournalStore();
  const { defaultCurrency, setLastJournalPath, lastJournalPath } = useSettingsStore();
  const [journals, setJournals] = useState<StoredJournal[] | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const refreshList = () =>
    api
      .listStoredJournals()
      .then(setJournals)
      .catch(() => setJournals([]));

  useEffect(() => {
    refreshList();
  }, []);

  const openPath = async (path: string, name: string): Promise<boolean> => {
    clearError();
    const ok =
      mode === "switch" ? await switchJournal(path) : await openJournal(path);
    if (ok) {
      // Persist a storage-relative ref; absolute container paths embed a UUID
      // that iOS changes on every app update.
      await setLastJournalPath(name || (await toPersistedJournalRef(path)));
      onOpened?.();
    }
    return ok;
  };

  const handleImport = async () => {
    setNotice(null);
    try {
      const selected = await open({
        // Multi-select so a journal's `include`d files come along. Relative
        // includes only resolve if the siblings sit next to the main file,
        // and iOS hands us one isolated copy per pick.
        multiple: true,
        // No filters: iOS uses UTIs not extensions, and .journal/.ledger
        // have no registered UTI so they'd be hidden with any filter.
      });
      const paths = (Array.isArray(selected) ? selected : selected ? [selected] : []) as string[];
      if (paths.length === 0) return;

      setBusy(true);
      const imported = [];
      for (const path of paths) {
        imported.push(await api.importJournalFile(path));
      }

      const renamed = imported.filter((r) => r.renamed);
      if (renamed.length > 0) {
        setNotice(
          `Already had a different journal by that name, so ${renamed
            .map((r) => r.fileName)
            .join(", ")} ${renamed.length === 1 ? "was" : "were"} imported under a new name.`
        );
        await refreshList();
        return;
      }

      // With several files there is no way to tell which is the main journal,
      // so list them and let the user open the right one.
      if (imported.length > 1) {
        setNotice(`Imported ${imported.length} files. Tap the main journal to open it.`);
        await refreshList();
        return;
      }

      await openPath(imported[0].path, imported[0].fileName);
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async (name: string) => {
    setNotice(null);
    setBusy(true);
    try {
      await api.deleteStoredJournal(name);
      if (lastJournalPath === name) await setLastJournalPath("");
      await refreshList();
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
      setConfirmDelete(null);
    }
  };

  const handleCreate = async () => {
    const name = newName.trim();
    if (!name) return;
    setNotice(null);
    setBusy(true);
    try {
      const created = await api.createStoredJournal(name, defaultCurrency);
      await openPath(created.path, created.fileName);
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
      // The name may already be taken; show what's there.
      await refreshList();
    } finally {
      setBusy(false);
    }
  };

  const fmtDate = (secs: number) =>
    secs > 0 ? new Date(secs * 1000).toLocaleDateString() : "";

  return (
    <div className="flex flex-col gap-3 w-full max-w-xs">
      {journals && journals.length > 0 && (
        <div className="rounded-lg border border-gray-200 dark:border-gray-700 divide-y divide-gray-100 dark:divide-gray-800 overflow-hidden">
          {journals.map((j) => (
            <div key={j.path} className="bg-white dark:bg-gray-800">
              <div className="flex items-stretch min-w-0">
                <button
                  onClick={() => openPath(j.path, j.name)}
                  disabled={busy}
                  className="flex-1 min-w-0 px-4 py-3 text-left active:bg-gray-50 dark:active:bg-gray-700 min-h-[48px]"
                >
                  <div className="text-sm font-medium text-gray-900 dark:text-gray-100 truncate">
                    {j.name}
                  </div>
                  <div className="text-xs text-gray-500 dark:text-gray-400">
                    {fmtDate(j.modified)}
                  </div>
                </button>
                <button
                  onClick={() => setConfirmDelete(confirmDelete === j.name ? null : j.name)}
                  disabled={busy}
                  aria-label={`Remove ${j.name}`}
                  className="px-4 shrink-0 text-gray-400 active:text-red-500 min-h-[48px]"
                >
                  &times;
                </button>
              </div>
              {confirmDelete === j.name && (
                <div className="px-4 pb-3 space-y-2">
                  <p className="text-xs text-gray-600 dark:text-gray-400 break-words">
                    Remove {j.name} from this app? The file is deleted from the
                    PocketHLedger folder; any copy elsewhere is untouched.
                  </p>
                  <div className="flex gap-2">
                    <button
                      onClick={() => handleDelete(j.name)}
                      disabled={busy}
                      className="flex-1 py-2 bg-red-600 text-white text-sm rounded-lg font-medium disabled:opacity-50 min-h-[44px]"
                    >
                      Remove
                    </button>
                    <button
                      onClick={() => setConfirmDelete(null)}
                      className="flex-1 py-2 text-sm text-gray-500 min-h-[44px]"
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {journals && journals.length === 0 && !creating && (
        <p className="text-xs text-gray-500 dark:text-gray-400 text-center">
          No journals yet. Import a journal file, or create a new one.
        </p>
      )}

      <button
        onClick={handleImport}
        disabled={busy}
        className="w-full px-6 py-3 bg-blue-600 text-white rounded-lg font-medium active:bg-blue-700 min-h-[48px] disabled:opacity-50"
      >
        {busy ? "Working..." : "Import Journal File…"}
      </button>
      <p className="text-xs text-gray-400 dark:text-gray-500 text-center -mt-1">
        Copies into this app, and shows up in the Files app under
        PocketHLedger. If your journal has <code>include</code> lines, select
        those files too — they must sit alongside it.
      </p>

      {showCreate &&
        (creating ? (
          <div className="flex gap-2">
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleCreate()}
              placeholder="finances"
              autoFocus
              className="flex-1 min-w-0 px-3 py-2 border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm"
            />
            <button
              onClick={handleCreate}
              disabled={busy || !newName.trim()}
              className="px-4 py-2 bg-blue-600 text-white text-sm rounded-lg font-medium disabled:opacity-50"
            >
              Create
            </button>
            <button
              onClick={() => {
                setCreating(false);
                setNewName("");
              }}
              className="px-3 py-2 text-gray-500 text-sm"
            >
              Cancel
            </button>
          </div>
        ) : (
          <button
            onClick={() => setCreating(true)}
            disabled={busy}
            className="w-full px-6 py-3 bg-white dark:bg-gray-800 text-blue-600 border border-blue-600 rounded-lg font-medium active:bg-blue-50 dark:active:bg-gray-700 min-h-[48px]"
          >
            Create New Journal
          </button>
        ))}

      {notice && (
        <div className="text-sm text-yellow-700 dark:text-yellow-400 bg-yellow-50 dark:bg-yellow-900/20 px-4 py-2 rounded-lg text-center break-words">
          {notice}
        </div>
      )}

      {error && (
        <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-4 py-2 rounded-lg text-center break-words">
          {error}
        </div>
      )}
    </div>
  );
}
