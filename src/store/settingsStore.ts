import { create } from "zustand";
import { load } from "@tauri-apps/plugin-store";

export type Theme = "light" | "dark" | "system";

interface SettingsState {
  defaultCurrency: string;
  theme: Theme;
  lastJournalPath: string | null;
  loaded: boolean;

  loadSettings: () => Promise<void>;
  setDefaultCurrency: (currency: string) => Promise<void>;
  setTheme: (theme: Theme) => Promise<void>;
  setLastJournalPath: (path: string) => Promise<void>;
}

const STORE_NAME = "settings.json";

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
  loaded: false,

  loadSettings: async () => {
    try {
      const store = await load(STORE_NAME);
      const currency = await store.get<string>("defaultCurrency");
      const theme = (await store.get<string>("theme")) as Theme | null;
      const lastPath = await store.get<string>("lastJournalPath");
      const resolvedTheme = theme ?? "system";
      applyTheme(resolvedTheme);
      set({
        defaultCurrency: currency ?? "$",
        theme: resolvedTheme,
        lastJournalPath: lastPath ?? null,
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
