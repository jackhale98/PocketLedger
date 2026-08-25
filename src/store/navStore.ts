import { create } from "zustand";
import type { TabId } from "../components/layout/BottomNav";

/** A one-shot instruction for the page being navigated to. The destination
 *  applies it and clears it, so going back doesn't re-trigger the jump. */
export type NavIntent =
  | { kind: "register"; account: string; dateFrom?: string; dateTo?: string }
  | { kind: "income-statement"; dateFrom?: string; dateTo?: string };

interface NavState {
  activeTab: TabId;
  intent: NavIntent | null;

  setActiveTab: (tab: TabId) => void;
  /** Switch tabs and hand the destination something to do on arrival. */
  navigate: (tab: TabId, intent: NavIntent) => void;
  clearIntent: () => void;
}

export const useNavStore = create<NavState>((set) => ({
  activeTab: "transactions",
  intent: null,

  setActiveTab: (tab) => set({ activeTab: tab, intent: null }),
  navigate: (tab, intent) => set({ activeTab: tab, intent }),
  clearIntent: () => set({ intent: null }),
}));
