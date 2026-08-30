import { create } from "zustand";
import type { ReportTab } from "./navStore";

/** View state for the Reports tab, kept outside the component so switching
 *  bottom tabs and coming back lands on the same report, account and dates
 *  -- the way the Accounts tab already remembers its filters. */
interface ReportsViewState {
  tab: ReportTab;
  registerAccount: string;
  dateFrom: string;
  dateTo: string;
  /** Applied to every report; the text box keeps its own draft and debounces
   *  into this. */
  query: string;
  showQuery: boolean;
  forecast: boolean;
  /** "" lets each chart size its buckets from the range; the rest pin it. */
  chartInterval: string;

  setTab: (tab: ReportTab) => void;
  setRegisterAccount: (account: string) => void;
  setDates: (from: string, to: string) => void;
  setQuery: (query: string) => void;
  setShowQuery: (show: boolean) => void;
  setForecast: (on: boolean) => void;
  setChartInterval: (interval: string) => void;
  /** Back to defaults: another journal has different accounts and dates. */
  reset: () => void;
}

const initial = {
  tab: "overview" as ReportTab,
  registerAccount: "",
  dateFrom: "",
  dateTo: "",
  query: "",
  showQuery: false,
  forecast: false,
  chartInterval: "",
};

export const useReportsViewStore = create<ReportsViewState>((set) => ({
  ...initial,
  setTab: (tab) => set({ tab }),
  setRegisterAccount: (registerAccount) => set({ registerAccount }),
  setDates: (dateFrom, dateTo) => set({ dateFrom, dateTo }),
  setQuery: (query) => set({ query }),
  setShowQuery: (showQuery) => set({ showQuery }),
  setForecast: (forecast) => set({ forecast }),
  setChartInterval: (chartInterval) => set({ chartInterval }),
  reset: () => set(initial),
}));
