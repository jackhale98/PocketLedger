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

  openJournal: (path: string) => Promise<void>;
  switchJournal: (path: string) => Promise<void>;
  addTransaction: (txn: NewTransaction) => Promise<void>;
  refresh: () => Promise<void>;
  clearError: () => void;
}

export const useJournalStore = create<JournalState>((set, get) => ({
  isLoaded: false,
  isLoading: false,
  error: null,
  summary: null,
  transactions: [],

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
      });
    } catch (err) {
      set({
        isLoading: false,
        error: err instanceof Error ? err.message : String(err),
      });
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
      });
    } catch (err) {
      set({
        isLoading: false,
        error: err instanceof Error ? err.message : String(err),
      });
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

  clearError: () => set({ error: null }),
}));
