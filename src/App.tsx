import { useState, useEffect } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { BottomNav, type TabId } from "./components/layout/BottomNav";
import { TransactionsPage } from "./pages/TransactionsPage";
import { AccountsPage } from "./pages/AccountsPage";
import { ReportsPage } from "./pages/ReportsPage";
import { MorePage } from "./pages/MorePage";
import { useJournalStore } from "./store/journalStore";
import { useSettingsStore } from "./store/settingsStore";
import * as api from "./api/commands";

function App() {
  const [activeTab, setActiveTab] = useState<TabId>("transactions");
  const { isLoaded, isLoading, error, summary, openJournal, clearError } =
    useJournalStore();
  const { defaultCurrency, loadSettings } = useSettingsStore();

  useEffect(() => {
    loadSettings();
  }, [loadSettings]);

  const handleOpenJournal = async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [
          {
            name: "Journal",
            extensions: ["journal", "hledger", "ledger", "j", "txt"],
          },
        ],
      });

      if (selected) {
        await openJournal(selected as string);
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
        await openJournal(path);
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
        <img src="/app-icon.svg" alt="PocketLedger" className="w-20 h-20 rounded-2xl" />
        <h1 className="text-2xl font-bold text-gray-900 dark:text-gray-100">PocketLedger</h1>
        <p className="text-gray-600 dark:text-gray-400 text-center">
          Plain text accounting in your pocket
        </p>
        {isLoading ? (
          <div className="text-sm text-gray-500">Loading...</div>
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
        {error && (
          <div className="text-sm text-red-600 bg-red-50 dark:bg-red-900/30 px-4 py-2 rounded-lg max-w-xs text-center">
            <p>{error}</p>
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

      {/* Error banner */}
      {error && (
        <div className="bg-red-50 dark:bg-red-900/30 px-4 py-2 flex items-center justify-between">
          <span className="text-sm text-red-600 dark:text-red-400">{error}</span>
          <button
            onClick={clearError}
            className="text-xs text-red-500 ml-2"
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
