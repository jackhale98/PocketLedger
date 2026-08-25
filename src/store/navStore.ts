import { create } from "zustand";
import type { TabId } from "../components/layout/BottomNav";

export type ReportTab = "overview" | "table" | "register" | "budget" | "forecast";

/** A one-shot instruction for the page being navigated to. The destination
 *  applies it and clears it, so going back doesn't re-trigger the jump. */
export type NavIntent =
  | { kind: "register"; account: string; dateFrom?: string; dateTo?: string }
  | { kind: "income-statement"; dateFrom?: string; dateTo?: string }
  | { kind: "report-tab"; tab: ReportTab };

/** Where the user was before a jump, so they can be put back. */
export interface Waypoint {
  tab: TabId;
  reportTab?: ReportTab;
}

interface NavState {
  activeTab: TabId;
  intent: NavIntent | null;
  history: Waypoint[];

  setActiveTab: (tab: TabId) => void;
  /** Switch tabs and hand the destination something to do on arrival.
   *  Passing `from` records a return point for [`goBack`]. */
  navigate: (tab: TabId, intent: NavIntent, from?: Waypoint) => void;
  goBack: () => void;
  clearIntent: () => void;
}

export const useNavStore = create<NavState>((set, get) => ({
  activeTab: "transactions",
  intent: null,
  history: [],

  // Choosing a tab from the bottom bar is a fresh start, not a step in a
  // drill-down, so it drops any pending return point.
  setActiveTab: (tab) => set({ activeTab: tab, intent: null, history: [] }),

  navigate: (tab, intent, from) =>
    set((s) => ({
      activeTab: tab,
      intent,
      history: from ? [...s.history, from] : s.history,
    })),

  goBack: () => {
    const { history } = get();
    const previous = history[history.length - 1];
    if (!previous) return;
    set({
      activeTab: previous.tab,
      intent: previous.reportTab
        ? { kind: "report-tab", tab: previous.reportTab }
        : null,
      history: history.slice(0, -1),
    });
  },

  clearIntent: () => set({ intent: null }),
}));
