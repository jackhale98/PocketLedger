import { useState, useEffect, useMemo } from "react";
import * as api from "../../api/commands";
import type { ReconciliationState } from "../../api/types";
import { useSettingsStore } from "../../store/settingsStore";

interface ReconciliationFlowProps {
  onDone: () => void;
  onEditTransaction?: (transactionIndex: number) => void;
}

type Step = "setup" | "reconcile";

export function ReconciliationFlow({ onDone, onEditTransaction }: ReconciliationFlowProps) {
  const { defaultCurrency } = useSettingsStore();
  const [step, setStep] = useState<Step>("setup");
  const [accounts, setAccounts] = useState<string[]>([]);
  const [account, setAccount] = useState("");
  const [statementDate, setStatementDate] = useState(new Date().toISOString().slice(0, 10));
  const [statementBalance, setStatementBalance] = useState("");
  const [commodity, setCommodity] = useState(defaultCurrency);
  const [reconcState, setReconcState] = useState<ReconciliationState | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [showCleared, setShowCleared] = useState(false);

  useEffect(() => {
    api.listAccountsWithBalances().then((data) => {
      setAccounts(data.map((a) => a.account).sort());
    });
  }, []);

  const handleStart = async () => {
    if (!account || !statementBalance) {
      setError("Account and statement balance are required");
      return;
    }
    setError(null);
    setLoading(true);
    try {
      const state = await api.startReconciliation({ account, statementDate, statementBalance, commodity });
      setReconcState(state);
      setStep("reconcile");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    setLoading(false);
  };

  const handleToggle = async (index: number) => {
    try {
      const state = await api.toggleReconciliationPosting(index);
      setReconcState(state);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleFinish = async () => {
    setLoading(true);
    try {
      await api.finishReconciliation();
      onDone();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setLoading(false);
    }
  };

  const handleCancel = async () => {
    await api.cancelReconciliation();
    onDone();
  };

  // Filtered postings for display
  const filteredPostings = useMemo(() => {
    if (!reconcState) return [];
    let result = reconcState.postings.map((p, i) => ({ ...p, originalIndex: i }));

    // Hide already-cleared unless toggled on
    if (!showCleared) {
      result = result.filter((p) => !p.isCleared);
    }

    // Search filter
    if (search.trim()) {
      const q = search.toLowerCase();
      result = result.filter((p) =>
        p.description.toLowerCase().includes(q) ||
        p.date.includes(q)
      );
    }

    // Most recent first
    result.reverse();

    return result;
  }, [reconcState, showCleared, search]);

  // Setup step
  if (step === "setup") {
    return (
      <div className="flex flex-col h-full bg-white dark:bg-gray-900">
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <button onClick={onDone} className="text-gray-600 dark:text-gray-300 text-sm font-medium">Cancel</button>
          <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Reconcile</h2>
          <button onClick={handleStart} disabled={loading}
            className="text-blue-600 text-sm font-semibold disabled:opacity-50">{loading ? "..." : "Start"}</button>
        </div>

        <div className="flex-1 overflow-auto p-4 space-y-4">
          {error && <div className="text-sm text-red-600 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">{error}</div>}

          <div>
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">Account</label>
            <select value={account} onChange={(e) => setAccount(e.target.value)}
              className="w-full px-3 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100">
              <option value="">Select account...</option>
              {accounts.map((a) => <option key={a} value={a}>{a}</option>)}
            </select>
          </div>

          <div>
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">Statement Date</label>
            <input type="date" value={statementDate} onChange={(e) => setStatementDate(e.target.value)}
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm" />
          </div>

          <div className="flex gap-2">
            <div className="flex-1">
              <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">Statement Balance</label>
              <input type="text" inputMode="decimal" value={statementBalance}
                onChange={(e) => setStatementBalance(e.target.value)} placeholder="e.g. 1234.56"
                className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm font-mono" />
            </div>
            <div className="w-20">
              <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">Currency</label>
              <input type="text" value={commodity} onChange={(e) => setCommodity(e.target.value)}
                className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm text-center" />
            </div>
          </div>
        </div>
      </div>
    );
  }

  if (!reconcState) return null;

  const diffColor = reconcState.isReconciled ? "text-green-500" : "text-red-500";

  return (
    <div className="flex flex-col h-full bg-white dark:bg-gray-900">
      {/* Header */}
      <div className="px-4 py-3 border-b border-gray-200 dark:border-gray-700 space-y-2">
        <div className="flex items-center justify-between">
          <button onClick={handleCancel} className="text-gray-600 dark:text-gray-300 text-sm">Cancel</button>
          <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">Reconcile</h2>
          <button onClick={handleFinish} disabled={loading || !reconcState.isReconciled}
            className="text-blue-600 text-sm font-semibold disabled:opacity-30">{loading ? "..." : "Finish"}</button>
        </div>

        {/* Summary */}
        <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 space-y-1">
          <div className="flex justify-between text-xs text-gray-500 dark:text-gray-400">
            <span>Statement</span>
            <span className="font-mono">{reconcState.statementCommodity}{reconcState.statementBalance}</span>
          </div>
          <div className="flex justify-between text-xs text-gray-500 dark:text-gray-400">
            <span>Cleared</span>
            <span className="font-mono">{reconcState.statementCommodity}{reconcState.clearedBalance}</span>
          </div>
          <div className={`flex justify-between text-sm font-semibold ${diffColor}`}>
            <span>Difference</span>
            <span className="font-mono">{reconcState.statementCommodity}{reconcState.difference}</span>
          </div>
          {reconcState.isReconciled && (
            <div className="text-center text-green-500 text-xs font-medium mt-1">Balanced! Tap Finish to save.</div>
          )}
        </div>

        {/* Search + toggle */}
        <div className="flex gap-2 items-center">
          <input type="text" value={search} onChange={(e) => setSearch(e.target.value)}
            placeholder="Search postings..."
            className="flex-1 px-3 py-2 bg-gray-100 dark:bg-gray-800 rounded-lg text-sm text-gray-900 dark:text-gray-100 placeholder-gray-500 dark:placeholder-gray-400" />
          <button onClick={() => setShowCleared(!showCleared)}
            className={`text-xs px-3 py-2 rounded-lg whitespace-nowrap ${
              showCleared ? "bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400" : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400"
            }`}>
            {showCleared ? "All" : "Uncleared"}
          </button>
        </div>
      </div>

      {error && <div className="mx-4 mt-2 text-sm text-red-600 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">{error}</div>}

      {/* Posting checklist */}
      <div className="flex-1 overflow-auto">
        <div className="divide-y divide-gray-100 dark:divide-gray-800">
          {filteredPostings.map((posting) => {
            const amount = parseFloat(posting.amount);
            return (
              <div key={`${posting.transactionIndex}-${posting.postingIndex}`}
                className="flex items-center min-h-[52px]">
                <button onClick={() => handleToggle(posting.originalIndex)}
                  className="px-4 py-3 flex items-center gap-3 flex-1 text-left active:bg-gray-50 dark:active:bg-gray-800">
                  <div className={`w-5 h-5 rounded border-2 flex items-center justify-center shrink-0 ${
                    posting.isCleared ? "bg-green-500 border-green-500" : "border-gray-300 dark:border-gray-600"}`}>
                    {posting.isCleared && <span className="text-white text-xs font-bold">&#10003;</span>}
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm text-gray-900 dark:text-gray-100 truncate">{posting.description}</div>
                    <div className="text-xs text-gray-500 dark:text-gray-400">{posting.date}</div>
                  </div>
                  <span className={`text-sm font-mono shrink-0 ${amount < 0 ? "text-red-500" : "text-green-500"}`}>
                    {posting.commodity}{Math.abs(amount).toFixed(2)}
                  </span>
                </button>
                {onEditTransaction && (
                  <button onClick={() => onEditTransaction(posting.transactionIndex)}
                    className="px-3 py-3 text-blue-500 text-xs shrink-0">
                    Edit
                  </button>
                )}
              </div>
            );
          })}
        </div>

        {filteredPostings.length === 0 && (
          <div className="text-center text-gray-500 dark:text-gray-400 text-sm py-8">
            {search ? "No matching postings" : showCleared ? "No postings" : "No uncleared postings"}
          </div>
        )}
      </div>
    </div>
  );
}
