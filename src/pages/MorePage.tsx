import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useSettingsStore, type Theme } from "../store/settingsStore";
import { useJournalStore } from "../store/journalStore";
import { ReconciliationFlow } from "../components/reconciliation/ReconciliationFlow";
import { BudgetEditor } from "../components/budget/BudgetEditor";

const COMMON_CURRENCIES = [
  { symbol: "$", label: "Dollar ($)" },
  { symbol: "\u20AC", label: "Euro (\u20AC)" },
  { symbol: "\u00A3", label: "Pound (\u00A3)" },
  { symbol: "\u00A5", label: "Yen/Yuan (\u00A5)" },
  { symbol: "\u20B9", label: "Rupee (\u20B9)" },
  { symbol: "\u20BF", label: "Bitcoin (\u20BF)" },
  { symbol: "CHF", label: "Swiss Franc (CHF)" },
  { symbol: "CAD", label: "Canadian Dollar (CAD)" },
  { symbol: "AUD", label: "Australian Dollar (AUD)" },
  { symbol: "USD", label: "US Dollar (USD)" },
];

const THEME_OPTIONS: { value: Theme; label: string }[] = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "System" },
];

export function MorePage() {
  const { defaultCurrency, setDefaultCurrency, theme, setTheme } = useSettingsStore();
  const { refresh, summary, switchJournal } = useJournalStore();
  const [customCurrency, setCustomCurrency] = useState("");
  const [showCustom, setShowCustom] = useState(false);
  const [showReconciliation, setShowReconciliation] = useState(false);
  const [showBudgetEditor, setShowBudgetEditor] = useState(false);

  const handleCustomSubmit = async () => {
    const val = customCurrency.trim();
    if (val) {
      await setDefaultCurrency(val);
      setShowCustom(false);
      setCustomCurrency("");
    }
  };

  const handleSwitchJournal = async () => {
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
        await switchJournal(selected as string);
      }
    } catch (err) {
      console.error("Switch journal error:", err);
    }
  };

  if (showReconciliation) {
    return (
      <ReconciliationFlow
        onDone={() => {
          setShowReconciliation(false);
          refresh();
        }}
      />
    );
  }

  if (showBudgetEditor) {
    return (
      <BudgetEditor
        onDone={() => {
          setShowBudgetEditor(false);
          refresh();
        }}
      />
    );
  }

  return (
    <div className="flex flex-col h-full">
      <div className="px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        <h1 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Settings</h1>
      </div>

      <div className="flex-1 overflow-auto">
        {/* Actions */}
        <div className="px-4 py-4 space-y-2">
          <h2 className="text-sm font-medium text-gray-700 dark:text-gray-300 uppercase tracking-wide mb-3">
            Tools
          </h2>
          <button
            onClick={() => setShowReconciliation(true)}
            className="w-full px-4 py-3 bg-gray-50 dark:bg-gray-800 rounded-lg text-sm text-left text-gray-900 dark:text-gray-100 active:bg-gray-100 dark:active:bg-gray-700 min-h-[48px] flex items-center justify-between"
          >
            <div>
              <div className="font-medium">Reconcile Account</div>
              <div className="text-xs text-gray-500 dark:text-gray-400">Match with bank statement</div>
            </div>
            <span className="text-gray-400">&rsaquo;</span>
          </button>
          <button
            onClick={() => setShowBudgetEditor(true)}
            className="w-full px-4 py-3 bg-gray-50 dark:bg-gray-800 rounded-lg text-sm text-left text-gray-900 dark:text-gray-100 active:bg-gray-100 dark:active:bg-gray-700 min-h-[48px] flex items-center justify-between"
          >
            <div>
              <div className="font-medium">Manage Budget</div>
              <div className="text-xs text-gray-500 dark:text-gray-400">Create or edit periodic budgets</div>
            </div>
            <span className="text-gray-400">&rsaquo;</span>
          </button>
          <button
            onClick={handleSwitchJournal}
            className="w-full px-4 py-3 bg-gray-50 dark:bg-gray-800 rounded-lg text-sm text-left text-gray-900 dark:text-gray-100 active:bg-gray-100 dark:active:bg-gray-700 min-h-[48px] flex items-center justify-between"
          >
            <div>
              <div className="font-medium">Switch Journal</div>
              <div className="text-xs text-gray-500 dark:text-gray-400">Open a different journal file</div>
            </div>
            <span className="text-gray-400">&rsaquo;</span>
          </button>
          <button
            onClick={() => refresh()}
            className="w-full px-4 py-3 bg-gray-50 dark:bg-gray-800 rounded-lg text-sm text-left text-gray-900 dark:text-gray-100 active:bg-gray-100 dark:active:bg-gray-700 min-h-[48px] flex items-center justify-between"
          >
            <div>
              <div className="font-medium">Reload Journal</div>
              <div className="text-xs text-gray-500 dark:text-gray-400">Re-read file from disk</div>
            </div>
            <span className="text-gray-400">&rsaquo;</span>
          </button>
        </div>

        <div className="h-2 bg-gray-100 dark:bg-gray-800" />

        {/* Theme */}
        <div className="px-4 py-4">
          <h2 className="text-sm font-medium text-gray-700 dark:text-gray-300 uppercase tracking-wide mb-3">
            Theme
          </h2>
          <div className="flex gap-2">
            {THEME_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setTheme(opt.value)}
                className={`flex-1 py-2.5 text-sm rounded-lg border min-h-[44px] ${
                  theme === opt.value
                    ? "border-blue-500 bg-blue-50 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400 font-medium"
                    : "border-gray-300 dark:border-gray-600 text-gray-600 dark:text-gray-400"
                }`}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </div>

        <div className="h-2 bg-gray-100 dark:bg-gray-800" />

        {/* Default Currency */}
        <div className="px-4 py-4">
          <h2 className="text-sm font-medium text-gray-700 dark:text-gray-300 uppercase tracking-wide mb-3">
            Default Currency
          </h2>
          <p className="text-xs text-gray-500 dark:text-gray-400 mb-3">
            Used as the default commodity for new transaction postings and chart values.
          </p>

          <div className="space-y-1">
            {COMMON_CURRENCIES.map((curr) => (
              <button
                key={curr.symbol}
                onClick={() => setDefaultCurrency(curr.symbol)}
                className={`w-full px-4 py-3 flex items-center justify-between rounded-lg text-sm min-h-[44px] ${
                  defaultCurrency === curr.symbol
                    ? "bg-blue-50 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400 font-medium"
                    : "text-gray-900 dark:text-gray-300 active:bg-gray-50 dark:active:bg-gray-800"
                }`}
              >
                <span>{curr.label}</span>
                {defaultCurrency === curr.symbol && (
                  <span className="text-blue-600 dark:text-blue-400 font-bold">&#10003;</span>
                )}
              </button>
            ))}

            {!showCustom ? (
              <button
                onClick={() => setShowCustom(true)}
                className={`w-full px-4 py-3 flex items-center justify-between rounded-lg text-sm min-h-[44px] ${
                  !COMMON_CURRENCIES.some((c) => c.symbol === defaultCurrency)
                    ? "bg-blue-50 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400 font-medium"
                    : "text-gray-500 dark:text-gray-400 active:bg-gray-50 dark:active:bg-gray-800"
                }`}
              >
                <span>
                  {!COMMON_CURRENCIES.some((c) => c.symbol === defaultCurrency)
                    ? `Custom: ${defaultCurrency}`
                    : "Other..."}
                </span>
                {!COMMON_CURRENCIES.some((c) => c.symbol === defaultCurrency) && (
                  <span className="text-blue-600 dark:text-blue-400 font-bold">&#10003;</span>
                )}
              </button>
            ) : (
              <div className="flex gap-2 px-4 py-2">
                <input
                  type="text"
                  value={customCurrency}
                  onChange={(e) => setCustomCurrency(e.target.value)}
                  placeholder="e.g. NOK, PLN, BRL"
                  className="flex-1 px-3 py-2 border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm"
                  autoFocus
                />
                <button onClick={handleCustomSubmit} className="px-4 py-2 bg-blue-600 text-white text-sm rounded-lg font-medium">
                  Set
                </button>
                <button onClick={() => { setShowCustom(false); setCustomCurrency(""); }} className="px-3 py-2 text-gray-500 text-sm">
                  Cancel
                </button>
              </div>
            )}
          </div>
        </div>

        <div className="h-2 bg-gray-100 dark:bg-gray-800" />

        {/* File info */}
        <div className="px-4 py-4">
          <h2 className="text-sm font-medium text-gray-700 dark:text-gray-300 uppercase tracking-wide mb-3">
            Current File
          </h2>
          <div className="text-sm text-gray-600 dark:text-gray-400 space-y-1">
            <p className="font-mono text-xs">{summary?.fileName ?? "No file loaded"}</p>
            {summary && (
              <p className="text-xs text-gray-500 dark:text-gray-400">
                {summary.transactionCount} transactions, {summary.accountCount} accounts
              </p>
            )}
          </div>
        </div>

        <div className="h-2 bg-gray-100 dark:bg-gray-800" />

        {/* About */}
        <div className="px-4 py-4">
          <h2 className="text-sm font-medium text-gray-700 dark:text-gray-300 uppercase tracking-wide mb-3">
            About
          </h2>
          <div className="text-sm text-gray-600 dark:text-gray-400 space-y-1">
            <p>hledger mobile v0.1.0</p>
            <p className="text-xs text-gray-400 dark:text-gray-500">
              Plain text accounting for Android &amp; iOS
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
