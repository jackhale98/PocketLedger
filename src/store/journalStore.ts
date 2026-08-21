import { create } from "zustand";
import type {
  JournalSummary,
  TransactionSummary,
  NewTransaction,
} from "../api/types";
import * as api from "../api/commands";

interface JournalState {
  isLoaded: boolean;
  isLoading: boolean;
  error: string | null;
  summary: JournalSummary | null;
  transactions: TransactionSummary[];
  /** Absolute path of the currently open journal (this launch only —
   *  never persist it; on iOS container paths change across updates). */
  currentPath: string | null;

  /** Returns true on success, false on failure (error state is set). */
  openJournal: (path: string) => Promise<boolean>;
  /** Returns true on success, false on failure (error state is set). */
  switchJournal: (path: string) => Promise<boolean>;
  addTransaction: (txn: NewTransaction) => Promise<void>;
  refresh: () => Promise<void>;
  /** Re-read the current journal from disk. Used when the app returns to the
   *  foreground, since a git client (e.g. Working Copy pulling into the app's
   *  documents folder) may have changed the file underneath us. */
  reloadFromDisk: () => Promise<void>;
  clearError: () => void;
}

export const useJournalStore = create<JournalState>((set, get) => ({
  isLoaded: false,
  isLoading: false,
  error: null,
  summary: null,
  transactions: [],
  currentPath: null,

  openJournal: async (path: string) => {
    set({ isLoading: true, error: null });
    try {
      const summary = await api.openJournal(path);
      const transactions = await api.listTransactions();
      set({
        isLoaded: true,
        isLoading: false,
        summary,
        transactions,
        currentPath: path,
      });
      return true;
    } catch (err) {
      set({
        isLoading: false,
        error: err instanceof Error ? err.message : String(err),
      });
      return false;
    }
  },

  switchJournal: async (path: string) => {
    set({ isLoading: true, error: null });
    try {
      const summary = await api.switchJournal(path);
      const transactions = await api.listTransactions();
      set({
        isLoaded: true,
        isLoading: false,
        summary,
        transactions,
        currentPath: path,
      });
      return true;
    } catch (err) {
      set({
        isLoading: false,
        error: err instanceof Error ? err.message : String(err),
      });
      return false;
    }
  },

  addTransaction: async (txn: NewTransaction) => {
    try {
      const summary = await api.addTransaction(txn);
      const transactions = await api.listTransactions();
      set({ summary, transactions, error: null });
    } catch (err) {
      set({
        error: err instanceof Error ? err.message : String(err),
      });
      throw err;
    }
  },

  refresh: async () => {
    if (!get().isLoaded) return;
    try {
      const summary = await api.getJournalInfo();
      const transactions = await api.listTransactions();
      set({ summary, transactions });
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
      set({ summary, transactions, error: null });
    } catch (err) {
      set({ error: err instanceof Error ? err.message : String(err) });
    }
  },

  clearError: () => set({ error: null }),
}));
