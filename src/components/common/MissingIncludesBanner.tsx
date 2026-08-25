import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import * as api from "../../api/commands";
import { useJournalStore } from "../../store/journalStore";

/** Shown when a journal's `include` lines point at files the app can't see.
 *  On iOS the picker hands over one file at a time, so importing a main
 *  journal alone leaves every include dangling — offer to fetch them. */
export function MissingIncludesBanner({
  missing,
  onDismiss,
}: {
  missing: string[];
  onDismiss: () => void;
}) {
  const { currentPath, openJournal } = useJournalStore();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleImport = async () => {
    setError(null);
    try {
      const selected = await open({ multiple: true });
      const paths = (Array.isArray(selected) ? selected : selected ? [selected] : []) as string[];
      if (paths.length === 0) return;

      setBusy(true);
      for (const path of paths) {
        await api.importJournalFile(path);
      }
      // Re-open so the includes resolve against the now-complete folder.
      if (currentPath) await openJournal(currentPath);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="bg-amber-50 dark:bg-amber-900/20 px-4 py-2 space-y-1.5">
      <div className="flex items-start justify-between gap-2">
        <div className="text-xs text-amber-800 dark:text-amber-300 break-words min-w-0 max-h-20 overflow-y-auto">
          {missing.length} included {missing.length === 1 ? "file is" : "files are"} missing:{" "}
          <span className="font-mono">{missing.join(", ")}</span>
        </div>
        <button onClick={onDismiss} className="text-xs text-amber-700 dark:text-amber-400 shrink-0 px-2 py-1">
          Dismiss
        </button>
      </div>
      <div className="text-xs text-amber-700/80 dark:text-amber-400/80 break-words">
        They need to sit next to your main journal in this app. Import them, or
        point Working Copy at the PocketHLedger folder so the whole repository
        lives here.
      </div>
      {error && (
        <div className="text-xs text-red-600 dark:text-red-400 break-words">{error}</div>
      )}
      <button
        onClick={handleImport}
        disabled={busy}
        className="w-full py-2 bg-amber-600 text-white text-xs font-medium rounded-lg disabled:opacity-50 min-h-[40px]"
      >
        {busy ? "Importing..." : "Import missing files…"}
      </button>
    </div>
  );
}
