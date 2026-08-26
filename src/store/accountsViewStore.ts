import { create } from "zustand";
import type { ValuationMode } from "../api/commands";

/** View state for the Accounts tab, kept outside the component so drilling
 *  into a register and coming back doesn't reset the filters and the tree
 *  the user had opened up. Only view state lives here; account data is
 *  reloaded from the journal. */
interface AccountsViewState {
  expanded: Set<string>;
  /** False until the tree has been auto-expanded once, so returning to the
   *  tab doesn't re-collapse what the user opened. */
  initialized: boolean;
  search: string;
  typeFilter: string;
  valueCurrency: string;
  /** Hide accounts whose balance is zero — long-closed accounts otherwise
   *  crowd a small screen. */
  hideZero: boolean;
  /** How balances are valued: what is held, what it cost, what it is worth, or
   *  the unrealised gain between the last two. */
  valuation: ValuationMode;

  setExpanded: (expanded: Set<string>) => void;
  /** Apply the default expansion, but only the first time. */
  initializeExpanded: (expanded: Set<string>) => void;
  setSearch: (search: string) => void;
  setTypeFilter: (typeFilter: string) => void;
  setValueCurrency: (valueCurrency: string) => void;
  setHideZero: (hideZero: boolean) => void;
  setValuation: (valuation: ValuationMode) => void;
}

export const useAccountsViewStore = create<AccountsViewState>((set) => ({
  expanded: new Set<string>(),
  initialized: false,
  search: "",
  typeFilter: "",
  valueCurrency: "",
  hideZero: false,
  valuation: "market",

  setExpanded: (expanded) => set({ expanded }),
  initializeExpanded: (expanded) =>
    set((s) => (s.initialized ? s : { ...s, expanded, initialized: true })),
  setSearch: (search) => set({ search }),
  setTypeFilter: (typeFilter) => set({ typeFilter }),
  setValueCurrency: (valueCurrency) => set({ valueCurrency }),
  setHideZero: (hideZero) => set({ hideZero }),
  setValuation: (valuation) => set({ valuation }),
}));
