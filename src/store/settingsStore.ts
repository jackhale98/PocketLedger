import { create } from "zustand";
import { load } from "@tauri-apps/plugin-store";
import { setIncognito } from "../utils/format";

export type Theme = "light" | "dark" | "system";

interface SettingsState {
  defaultCurrency: string;
  theme: Theme;
  lastJournalPath: string | null;
  /** Mask every amount, for using the app where others can see the screen. */
  incognito: boolean;
  loaded: boolean;

  loadSettings: () => Promise<void>;
  setDefaultCurrency: (currency: string) => Promise<void>;
  setTheme: (theme: Theme) => Promise<void>;
  setLastJournalPath: (path: string) => Promise<void>;
  setIncognito: (on: boolean) => Promise<void>;
}

const STORE_NAME = "settings.json";

/** Amount text is masked by the formatter; charts are blurred by CSS, since
 *  their axis labels and tooltips are drawn by the chart library. */
function applyIncognito(on: boolean) {
  setIncognito(on);
  document.documentElement.classList.toggle("incognito", on);
}

function applyTheme(theme: Theme) {
  const root = document.documentElement;
  if (theme === "dark") {
    root.classList.add("dark");
  } else if (theme === "light") {
    root.classList.remove("dark");
  } else {
    // system
    if (window.matchMedia("(prefers-color-scheme: dark)").matches) {
      root.classList.add("dark");
    } else {
      root.classList.remove("dark");
    }
  }
}

export const useSettingsStore = create<SettingsState>((set) => ({
  defaultCurrency: "$",
  theme: "system",
  lastJournalPath: null,
  incognito: false,
  loaded: false,

  loadSettings: async () => {
    try {
      const store = await load(STORE_NAME);
      const currency = await store.get<string>("defaultCurrency");
      const theme = (await store.get<string>("theme")) as Theme | null;
      const lastPath = await store.get<string>("lastJournalPath");
      const incognito = (await store.get<boolean>("incognito")) ?? false;
      const resolvedTheme = theme ?? "system";
      applyTheme(resolvedTheme);
      applyIncognito(incognito);
      set({
        defaultCurrency: currency ?? "$",
        theme: resolvedTheme,
        lastJournalPath: lastPath ?? null,
        incognito,
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
