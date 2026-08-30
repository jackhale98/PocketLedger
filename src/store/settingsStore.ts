import { create } from "zustand";
import { load } from "@tauri-apps/plugin-store";
import { setIncognito } from "../utils/format";
import * as api from "../api/commands";

export type Theme = "light" | "dark" | "system";

interface SettingsState {
  defaultCurrency: string;
  theme: Theme;
  lastJournalPath: string | null;
  /** Mask every amount, for using the app where others can see the screen. */
  incognito: boolean;
  /** Use transaction costs (`@`) as market prices, like hledger's
   *  --infer-market-prices. Off matches the CLI's defaults. */
  inferMarketPrices: boolean;
  loaded: boolean;

  loadSettings: () => Promise<void>;
  setDefaultCurrency: (currency: string) => Promise<void>;
  setTheme: (theme: Theme) => Promise<void>;
  setLastJournalPath: (path: string) => Promise<void>;
  setIncognito: (on: boolean) => Promise<void>;
  setInferMarketPrices: (on: boolean) => Promise<void>;
}

const STORE_NAME = "settings.json";

/** Amount text is masked by the formatter; charts are blurred by CSS, since
 *  their axis labels and tooltips are drawn by the chart library. */
function applyIncognito(on: boolean) {
  setIncognito(on);
  document.documentElement.classList.toggle("incognito", on);
}

const systemDark = (): MediaQueryList | null =>
  typeof window !== "undefined" && window.matchMedia
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;

/** While the theme is "system", follow the OS as it changes -- evaluating
 *  the media query once at startup left the app in the wrong theme after
 *  the phone flipped to dark at sunset. */
let unfollowSystem: (() => void) | null = null;

function applyTheme(theme: Theme) {
  const root = document.documentElement;
  unfollowSystem?.();
  unfollowSystem = null;
  if (theme === "dark") {
    root.classList.add("dark");
  } else if (theme === "light") {
    root.classList.remove("dark");
  } else {
    const mq = systemDark();
    const sync = () => root.classList.toggle("dark", mq?.matches ?? false);
    sync();
    if (mq) {
      mq.addEventListener("change", sync);
      unfollowSystem = () => mq.removeEventListener("change", sync);
    }
  }
}

export const useSettingsStore = create<SettingsState>((set) => ({
  defaultCurrency: "$",
  theme: "system",
  lastJournalPath: null,
  incognito: false,
  inferMarketPrices: false,
  loaded: false,

  loadSettings: async () => {
    try {
      const store = await load(STORE_NAME);
      const currency = await store.get<string>("defaultCurrency");
      const theme = (await store.get<string>("theme")) as Theme | null;
      const lastPath = await store.get<string>("lastJournalPath");
      const incognito = (await store.get<boolean>("incognito")) ?? false;
      const inferMarketPrices = (await store.get<boolean>("inferMarketPrices")) ?? false;
      if (inferMarketPrices) {
        // Must reach the engine before the journal opens; failure only
        // means the CLI default applies.
        api.setEngineOptions({ inferMarketPrices }).catch(() => {});
      }
      const resolvedTheme = theme ?? "system";
      applyTheme(resolvedTheme);
      applyIncognito(incognito);
      set({
        defaultCurrency: currency ?? "$",
        theme: resolvedTheme,
        lastJournalPath: lastPath ?? null,
        incognito,
        inferMarketPrices,
        loaded: true,
      });
    } catch {
      applyTheme("system");
      set({ loaded: true });
    }
  },

  setDefaultCurrency: async (currency: string) => {
    set({ defaultCurrency: currency });
    try {
      const store = await load(STORE_NAME);
      await store.set("defaultCurrency", currency);
      await store.save();
    } catch (err) {
      console.error("Failed to save settings:", err);
    }
  },

  setLastJournalPath: async (path: string) => {
    set({ lastJournalPath: path });
    try {
      const store = await load(STORE_NAME);
      await store.set("lastJournalPath", path);
      await store.save();
    } catch (err) {
      console.error("Failed to save last journal path:", err);
    }
  },

  setInferMarketPrices: async (on: boolean) => {
    set({ inferMarketPrices: on });
    try {
      const store = await load(STORE_NAME);
      await store.set("inferMarketPrices", on);
      await store.save();
    } catch (err) {
      console.error("Failed to save valuation setting:", err);
    }
    // Applying reloads the open journal; the store notices via the summary.
    try {
      const summary = await api.setEngineOptions({ inferMarketPrices: on });
      if (summary) {
        const { useJournalStore } = await import("./journalStore");
        await useJournalStore.getState().refresh();
      }
    } catch (err) {
      console.error("Failed to apply valuation setting:", err);
    }
  },

  setIncognito: async (on: boolean) => {
    applyIncognito(on);
    set({ incognito: on });
    try {
      const store = await load(STORE_NAME);
      await store.set("incognito", on);
      await store.save();
    } catch (err) {
      console.error("Failed to save incognito setting:", err);
    }
  },

  setTheme: async (theme: Theme) => {
    applyTheme(theme);
    set({ theme });
    try {
      const store = await load(STORE_NAME);
      await store.set("theme", theme);
      await store.save();
    } catch (err) {
      console.error("Failed to save settings:", err);
    }
  },
}));
