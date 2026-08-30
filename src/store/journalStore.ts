import { create } from "zustand";
import type {
  JournalSummary,
  TransactionSummary,
  NewTransaction,
} from "../api/types";
import * as api from "../api/commands";
import { useAccountsViewStore } from "./accountsViewStore";
import { useReportsViewStore } from "./reportsViewStore";

interface JournalState {
  isLoaded: boolean;
  isLoading: boolean;
  error: string | null;
  summary: JournalSummary | null;
  transactions: TransactionSummary[];
  /** Absolute path of the currently open journal (this launch only —
   *  never persist it; on iOS container paths change across updates). */
  currentPath: string | null;
  /** Bumped every time the transaction list is replaced from the backend, so
   *  a screen holding an index into it can notice the ground moved. */
  loadGeneration: number;

  /** Returns true on success, false on failure (error state is set). */
  openJournal: (path: string) => Promise<boolean>;
  /** Returns true on success, false on failure (error state is set). */
  switchJournal: (path: string) => Promise<boolean>;
  /** Rejects on failure; the caller shows the message inline, so the global
   *  banner stays quiet. */
  addTransaction: (txn: NewTransaction, fileIndex?: number) => Promise<void>;
  refresh: () => Promise<void>;
  /** Re-read the current journal from disk. Used when the app returns to the
   *  foreground, since a git client (e.g. Working Copy pulling into the app's
   *  documents folder) may have changed the file underneath us. */
  reloadFromDisk: () => Promise<void>;
  /** Write the loaded journal back to a path that vanished on disk. */
  recreateFromMemory: () => Promise<boolean>;
  clearError: () => void;
}

/** Per-journal view state (opened tree, filters, chosen accounts) means
 *  nothing once a different journal is loaded. */
function resetViewStores() {
  useAccountsViewStore.getState().reset();
  useReportsViewStore.getState().reset();
}

export const useJournalStore = create<JournalState>((set, get) => {
  /** open_journal and switch_journal differ only in which command runs; the
   *  bookkeeping around them is identical. */
  const load = async (
    path: string,
    command: (path: string) => Promise<JournalSummary>
  ): Promise<boolean> => {
    const switching = get().currentPath !== null && get().currentPath !== path;
    set({ isLoading: true, error: null });
    try {
      const summary = await command(path);
      const transactions = await api.listTransactions();
      if (switching || get().currentPath === null) resetViewStores();
      set((s) => ({
        isLoaded: true,
        isLoading: false,
        summary,
        transactions,
        currentPath: path,
        loadGeneration: s.loadGeneration + 1,
      }));
      return true;
    } catch (err) {
      set({
        isLoading: false,
        error: err instanceof Error ? err.message : String(err),
      });
      return false;
    }
  };

  return {
    isLoaded: false,
    isLoading: false,
    error: null,
    summary: null,
    transactions: [],
    currentPath: null,
    loadGeneration: 0,

    openJournal: (path) => load(path, api.openJournal),
    switchJournal: (path) => load(path, api.switchJournal),

    addTransaction: async (txn, fileIndex) => {
      const summary = await api.addTransaction(txn, fileIndex);
      const transactions = await api.listTransactions();
      set((s) => ({ summary, transactions, loadGeneration: s.loadGeneration + 1 }));
    },

    refresh: async () => {
      if (!get().isLoaded) return;
      try {
        const summary = await api.getJournalInfo();
        const transactions = await api.listTransactions();
        set((s) => ({ summary, transactions, loadGeneration: s.loadGeneration + 1 }));
      } catch (err) {
        set({
          error: err instanceof Error ? err.message : String(err),
        });
      }
    },

    reloadFromDisk: async () => {
      const { isLoaded, currentPath } = get();
      if (!isLoaded || !currentPath) return;
      try {
        // Only reload when the file really changed — a needless reload would
        // invalidate an in-progress reconciliation session.
        if (!(await api.journalChangedOnDisk())) return;
        const summary = await api.openJournal(currentPath);
        const transactions = await api.listTransactions();
        set((s) => ({
          summary,
          transactions,
          error: null,
          loadGeneration: s.loadGeneration + 1,
        }));
      } catch (err) {
        set({ error: err instanceof Error ? err.message : String(err) });
      }
    },

    recreateFromMemory: async () => {
      try {
        const summary = await api.recreateJournalFromMemory();
        const transactions = await api.listTransactions();
        set((s) => ({
          summary,
          transactions,
          error: null,
          loadGeneration: s.loadGeneration + 1,
        }));
        return true;
      } catch (err) {
        set({ error: err instanceof Error ? err.message : String(err) });
        return false;
      }
    },

    clearError: () => set({ error: null }),
  };
});
