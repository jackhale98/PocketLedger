import { useState, useEffect, useRef } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { BottomNav, type TabId } from "./components/layout/BottomNav";
import { TransactionsPage } from "./pages/TransactionsPage";
import { AccountsPage } from "./pages/AccountsPage";
import { ReportsPage } from "./pages/ReportsPage";
import { MorePage } from "./pages/MorePage";
import { MobileJournalPicker } from "./components/common/MobileJournalPicker";
import { useJournalStore } from "./store/journalStore";
import { useSettingsStore } from "./store/settingsStore";
import { getPlatformInfo, resolveJournalRef } from "./utils/platform";
import * as api from "./api/commands";

function App() {
  const [activeTab, setActiveTab] = useState<TabId>("transactions");
  const { isLoaded, isLoading, error, summary, openJournal, reloadFromDisk, clearError } =
    useJournalStore();
  const { defaultCurrency, lastJournalPath, loaded: settingsLoaded, loadSettings, setLastJournalPath } = useSettingsStore();
  const [isMobile, setIsMobile] = useState<boolean | null>(null);

  useEffect(() => {
    loadSettings();
    getPlatformInfo().then((info) => setIsMobile(info.isMobile));
  }, [loadSettings]);

  // Auto-open last journal on startup (attempt once — never retry-loop).
  // The persisted ref is a bare file name on mobile; resolve it against the
  // app's storage dir, which changes across iOS app updates.
  const autoOpenAttempted = useRef(false);
  const [autoOpenFailedPath, setAutoOpenFailedPath] = useState<string | null>(null);
  useEffect(() => {
    if (settingsLoaded && isMobile !== null && !isLoaded && !isLoading && lastJournalPath && !autoOpenAttempted.current) {
      autoOpenAttempted.current = true;
      const ref = lastJournalPath;
      resolveJournalRef(ref)
        .then((path) => openJournal(path))
        .then((ok) => {
          if (!ok) {
            // File may have been moved/deleted - clear the saved path
            setAutoOpenFailedPath(ref);
            setLastJournalPath("");
          }
        });
    }
  }, [settingsLoaded, isMobile, isLoaded, isLoading, lastJournalPath, openJournal, setLastJournalPath]);

  // Pick up outside changes when the app returns to the foreground: a git client
  // (Working Copy linked to the journal folder) may have pulled new commits.
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === "visible") reloadFromDisk();
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onVisible);
    return () => {
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onVisible);
    };
  }, [reloadFromDisk]);

  const handleOpenJournal = async () => {
    try {
      const selected = await open({
        multiple: false,
        // No filters: iOS uses UTIs not extensions, and .journal/.ledger
        // have no registered UTI so they'd be hidden with any filter.
      });

      if (selected) {
        const path = selected as string;
        const ok = await openJournal(path);
        if (ok) {
          setAutoOpenFailedPath(null);
          await setLastJournalPath(path);
        }
      }
    } catch (err) {
      console.error("File picker error:", err);
    }
  };

  const handleCreateJournal = async () => {
    try {
      const selected = await save({
        filters: [
          {
            name: "Journal",
            extensions: ["journal"],
          },
        ],
        defaultPath: "finances.journal",
      });

      if (selected) {
        let path = selected as string;
        if (!path.endsWith(".journal")) {
          path += ".journal";
        }
        await api.createJournal(path, defaultCurrency);
        const ok = await openJournal(path);
        if (ok) {
          setAutoOpenFailedPath(null);
          await setLastJournalPath(path);
        }
      }
    } catch (err) {
      console.error("Create journal error:", err);
    }
  };

  const renderPage = () => {
    switch (activeTab) {
      case "transactions":
        return <TransactionsPage />;
      case "accounts":
        return <AccountsPage />;
      case "reports":
        return <ReportsPage />;
      case "more":
        return <MorePage />;
    }
  };

  if (!isLoaded) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-4 p-8 bg-white dark:bg-gray-900">
        <img src="/app-icon.svg" alt="PocketHLedger" className="w-20 h-20 rounded-2xl" />
        <h1 className="text-2xl font-bold text-gray-900 dark:text-gray-100">PocketHLedger</h1>
        <p className="text-gray-600 dark:text-gray-400 text-center">
          Plain text accounting in your pocket
        </p>
        {isLoading || isMobile === null ? (
          <div className="text-sm text-gray-500">Loading...</div>
        ) : isMobile ? (
          <MobileJournalPicker
            mode="open"
            showCreate
            onOpened={() => setAutoOpenFailedPath(null)}
          />
        ) : (
          <div className="flex flex-col gap-3 w-full max-w-xs">
            <button
              onClick={handleOpenJournal}
              className="w-full px-6 py-3 bg-blue-600 text-white rounded-lg font-medium active:bg-blue-700 min-h-[48px]"
            >
              Open Journal
            </button>
            <button
              onClick={handleCreateJournal}
              className="w-full px-6 py-3 bg-white dark:bg-gray-800 text-blue-600 border border-blue-600 rounded-lg font-medium active:bg-blue-50 dark:active:bg-gray-700 min-h-[48px]"
            >
              Create New Journal
            </button>
          </div>
        )}
        {autoOpenFailedPath && (
          <div className="text-sm text-yellow-700 dark:text-yellow-400 bg-yellow-50 dark:bg-yellow-900/20 px-4 py-2 rounded-lg max-w-xs text-center">
            <p>Couldn't reopen the last journal:</p>
            <p className="font-mono text-xs break-all mt-1">{autoOpenFailedPath}</p>
            <p className="text-xs mt-1">Pick another journal below.</p>
          </div>
        )}
        {error && (
          <div className="text-sm text-red-600 bg-red-50 dark:bg-red-900/30 px-4 py-2 rounded-lg max-w-xs text-center">
            <p className="break-words">{error}</p>
            <button
              onClick={clearError}
              className="mt-2 text-xs text-red-500 underline"
            >
              Dismiss
            </button>
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full bg-white dark:bg-gray-900">
      {/* Status bar */}
      <div className="bg-white dark:bg-gray-900 border-b border-gray-100 dark:border-gray-800 px-4 py-1 pt-safe-top">
        <div className="text-xs text-gray-500 dark:text-gray-400 text-center">
          {summary?.fileName} &middot; {summary?.transactionCount} transactions
        </div>
      </div>

      {/* Warning banner for include failures */}
      {summary?.warnings && summary.warnings.length > 0 && (
        <div className="bg-yellow-50 dark:bg-yellow-900/20 px-4 py-1.5">
          {summary.warnings.map((w, i) => (
            <div key={i} className="text-xs text-yellow-700 dark:text-yellow-400">{w}</div>
          ))}
        </div>
      )}

      {/* Error banner */}
      {error && (
        <div className="bg-red-50 dark:bg-red-900/30 px-4 py-2 flex items-start justify-between gap-2">
          <span className="text-sm text-red-600 dark:text-red-400 min-w-0 break-words">{error}</span>
          <button
            onClick={clearError}
            className="text-xs text-red-500 ml-2 shrink-0"
          >
            Dismiss
          </button>
        </div>
      )}

      {/* Main content */}
      <main className="flex-1 overflow-hidden bg-white dark:bg-gray-900">{renderPage()}</main>

      {/* Bottom navigation */}
      <BottomNav activeTab={activeTab} onTabChange={setActiveTab} />
    </div>
  );
}

export default App;
