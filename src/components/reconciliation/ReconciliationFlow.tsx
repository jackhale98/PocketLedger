import { useState, useEffect, useMemo, useRef } from "react";
import { format } from "date-fns";
import * as api from "../../api/commands";
import { normalizeAmountInput } from "../../utils/amount";
import type { ReconciliationState } from "../../api/types";
import { useSettingsStore } from "../../store/settingsStore";
import { useBackHandler } from "../../store/backStore";
import { amountTone, formatAmount } from "../../utils/format";
import { SignToggle } from "../common/SignToggle";

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
  const [statementDate, setStatementDate] = useState(format(new Date(), "yyyy-MM-dd"));
  const [statementBalance, setStatementBalance] = useState("");
  const [commodity, setCommodity] = useState(defaultCurrency);
  const [reconcState, setReconcState] = useState<ReconciliationState | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [showCleared, setShowCleared] = useState(false);
  const [confirmForce, setConfirmForce] = useState(false);
  /** Pin the statement balance with a balance assertion on finish — the
   *  plain-text-accounting habit that makes later drift show up in
   *  `hledger check` and in this app's warnings. */
  const [addAssertion, setAddAssertion] = useState(true);

  const toggleSeq = useRef(0);

  useEffect(() => {
    api
      .listAccountsWithBalances()
      .then((data) => {
        setAccounts(data.map((a) => a.account).sort());
      })
      .catch((err) => {
        setError(err instanceof Error ? err.message : String(err));
      });
  }, []);

  const handleStart = async () => {
    if (!account || !statementBalance) {
      setError("Account and statement balance are required");
      return;
    }
    const normalizedBalance = normalizeAmountInput(statementBalance);
    if (normalizedBalance === null) {
      setError(`"${statementBalance}" is not a valid amount`);
      return;
    }
    setError(null);
    setLoading(true);
    try {
      const state = await api.startReconciliation({ account, statementDate, statementBalance: normalizedBalance, commodity });
      setReconcState(state);
      setStep("reconcile");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  const handleToggle = async (index: number) => {
    const seq = ++toggleSeq.current;
    try {
      const state = await api.toggleReconciliationPosting(index);
      // Ignore stale responses from rapid toggling
      if (seq !== toggleSeq.current) return;
      setReconcState(state);
    } catch (err) {
      if (seq !== toggleSeq.current) return;
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  /** Save the cleared marks. `force` writes them even though the cleared
   *  total disagrees with the statement, for a partial pass. */
  const handleFinish = async (force = false) => {
    setLoading(true);
    setConfirmForce(false);
    try {
      await api.finishReconciliation(force || undefined, addAssertion);
      onDone();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setLoading(false);
    }
  };

  /** Drop the backend session and go back to the setup step. */
  const abandonSession = async () => {
    try {
      await api.cancelReconciliation();
    } catch (err) {
      // Session cleanup failure shouldn't trap the user in this screen
      console.error("Cancel reconciliation error:", err);
    }
  };

  const handleCancel = async () => {
    await abandonSession();
    onDone();
  };

  // Back steps the same way the buttons do: a confirmation closes first, the
  // checklist returns to setup (dropping the session so it isn't left open),
  // and setup leaves the flow. Without this a swipe paged to another tab
  // with the backend session still open.
  useBackHandler(true, () => {
    if (confirmForce) {
      setConfirmForce(false);
    } else if (step === "reconcile") {
      abandonSession().then(() => {
        setReconcState(null);
        setStep("setup");
      });
    } else {
      onDone();
    }
  });

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
          <button onClick={onDone} className="text-gray-600 dark:text-gray-300 text-sm font-medium min-h-[44px] min-w-[60px] text-left">Cancel</button>
          <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Reconcile</h2>
          <button onClick={handleStart} disabled={loading}
            className="text-blue-600 text-sm font-semibold disabled:opacity-50 min-h-[44px] min-w-[60px] text-right">{loading ? "..." : "Start"}</button>
        </div>

        <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 space-y-4">
          {error && <div className="text-sm text-red-600 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">{error}</div>}

          <div>
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">Account</label>
            <select value={account} onChange={(e) => setAccount(e.target.value)}
              aria-label="Account"
              className="w-full px-3 py-2 min-h-[44px] bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100">
              <option value="">Select account...</option>
              {accounts.map((a) => <option key={a} value={a}>{a}</option>)}
            </select>
          </div>

          <div>
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">Statement Date</label>
            <input type="date" value={statementDate} onChange={(e) => setStatementDate(e.target.value)}
              aria-label="Statement date"
              className="w-full px-3 py-2 min-h-[44px] border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm" />
          </div>

          <div className="flex gap-2 items-end min-w-0">
            <div className="flex-1 min-w-0">
              <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">Statement Balance</label>
              <input type="text" inputMode="decimal" value={statementBalance}
                autoCapitalize="none" autoCorrect="off" spellCheck={false} enterKeyHint="done"
                aria-label="Statement balance"
                onChange={(e) => setStatementBalance(e.target.value)} placeholder="e.g. 1234.56"
                className="w-full px-3 py-2 min-h-[44px] border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm font-mono" />
            </div>
            <SignToggle value={statementBalance} onChange={setStatementBalance} className="dark:bg-gray-800" />
            <div className="w-16 shrink-0">
              <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">Currency</label>
              <input type="text" value={commodity} onChange={(e) => setCommodity(e.target.value)}
                autoCapitalize="none" autoCorrect="off" spellCheck={false}
                aria-label="Currency"
                className="w-full px-1 py-2 min-h-[44px] border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm text-center" />
            </div>
          </div>
        </div>
      </div>
    );
  }

  if (!reconcState) return null;

  const diffColor = reconcState.isReconciled ? "text-positive" : "text-negative";

  return (
    <div className="flex flex-col h-full bg-white dark:bg-gray-900">
      {/* Header */}
      <div className="px-4 py-3 border-b border-gray-200 dark:border-gray-700 space-y-2">
        <div className="flex items-center justify-between">
          <button onClick={handleCancel} className="text-gray-600 dark:text-gray-300 text-sm min-h-[44px] min-w-[60px] text-left">Cancel</button>
          <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">Reconcile</h2>
          {reconcState.isReconciled ? (
            <button onClick={() => handleFinish()} disabled={loading}
              className="text-blue-600 text-sm font-semibold disabled:opacity-30 min-h-[44px] min-w-[60px] text-right">{loading ? "..." : "Finish"}</button>
          ) : (
            <button onClick={() => setConfirmForce(true)} disabled={loading}
              title="Save the cleared marks even though the totals differ"
              className="text-amber-600 dark:text-amber-400 text-xs font-semibold disabled:opacity-30 min-h-[44px] min-w-[60px] text-right">{loading ? "..." : "Finish anyway"}</button>
          )}
        </div>

        {confirmForce && (
          <div className="bg-amber-50 dark:bg-amber-900/20 rounded-lg p-3 space-y-2">
            <p className="text-xs text-amber-800 dark:text-amber-300 break-words">
              The cleared total is off by {formatAmount(reconcState.difference, reconcState.statementCommodity)}.
              Mark the ticked postings as cleared anyway? You can finish reconciling later.
            </p>
            <div className="flex gap-2">
              <button onClick={() => setConfirmForce(false)}
                className="flex-1 min-h-[44px] text-sm font-medium text-gray-600 dark:text-gray-400 rounded-lg border border-gray-300 dark:border-gray-600">
                Keep going
              </button>
              <button onClick={() => handleFinish(true)} disabled={loading}
                className="flex-1 min-h-[44px] text-sm font-medium text-white bg-amber-600 rounded-lg active:bg-amber-700 disabled:opacity-50">
                Save anyway
              </button>
            </div>
          </div>
        )}

        {/* Summary */}
        <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 space-y-1">
          <div className="flex justify-between text-xs text-gray-500 dark:text-gray-400">
            <span>Statement</span>
            <span className="font-mono">{formatAmount(reconcState.statementBalance, reconcState.statementCommodity)}</span>
          </div>
          <div className="flex justify-between text-xs text-gray-500 dark:text-gray-400">
            <span>Cleared</span>
            <span className="font-mono">{formatAmount(reconcState.clearedBalance, reconcState.statementCommodity)}</span>
          </div>
          <div className={`flex justify-between text-sm font-semibold ${diffColor}`}>
            <span>Difference</span>
            <span className="font-mono">{formatAmount(reconcState.difference, reconcState.statementCommodity)}</span>
          </div>
          {reconcState.isReconciled && (
            <>
              <div className="text-center text-positive text-xs font-medium mt-1">Balanced! Tap Finish to save.</div>
              <label className="flex items-center gap-2 text-xs text-gray-600 dark:text-gray-400 min-h-[44px] cursor-pointer">
                <input
                  type="checkbox"
                  checked={addAssertion}
                  onChange={(e) => setAddAssertion(e.target.checked)}
                  className="w-5 h-5 accent-blue-600"
                />
                <span>
                  Record a balance assertion ({formatAmount(reconcState.statementBalance, reconcState.statementCommodity)} on {reconcState.statementDate})
                </span>
              </label>
            </>
          )}
        </div>

        {/* Search + toggle */}
        <div className="flex gap-2 items-center">
          <input type="text" value={search} onChange={(e) => setSearch(e.target.value)}
            placeholder="Search postings..."
            aria-label="Search postings"
            autoCapitalize="none" autoCorrect="off" spellCheck={false} enterKeyHint="search"
            className="flex-1 min-w-0 px-3 py-2 min-h-[44px] bg-gray-100 dark:bg-gray-800 rounded-lg text-sm text-gray-900 dark:text-gray-100 placeholder-gray-500 dark:placeholder-gray-400" />
          <button onClick={() => setShowCleared(!showCleared)}
            aria-pressed={showCleared}
            className={`text-xs px-3 py-2 min-h-[44px] rounded-lg whitespace-nowrap ${
              showCleared ? "bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400" : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400"
            }`}>
            {showCleared ? "All" : "Uncleared"}
          </button>
        </div>
      </div>

      {error && <div className="mx-4 mt-2 text-sm text-red-600 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">{error}</div>}

      {/* Posting checklist */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden">
        <div className="divide-y divide-gray-100 dark:divide-gray-800">
          {filteredPostings.map((posting) => {
            return (
              <div key={`${posting.transactionIndex}-${posting.postingIndex}`}
                className="flex items-center min-h-[52px]">
                <button onClick={() => handleToggle(posting.originalIndex)}
                  role="checkbox" aria-checked={posting.isCleared}
                  className="px-4 py-3 flex items-center gap-3 flex-1 min-w-0 text-left active:bg-gray-50 dark:active:bg-gray-800">
                  <div className={`w-5 h-5 rounded border-2 flex items-center justify-center shrink-0 ${
                    posting.isCleared ? "bg-green-500 border-green-500" : "border-gray-300 dark:border-gray-600"}`}>
                    {posting.isCleared && <span className="text-white text-xs font-bold">&#10003;</span>}
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm text-gray-900 dark:text-gray-100 truncate" title={posting.description}>{posting.description}</div>
                    <div className="text-xs text-gray-500 dark:text-gray-400">{posting.date}</div>
                  </div>
                  {/* Through the shared formatter: commodity precision, and
                      the hide-amounts setting, which a raw toFixed leaked. */}
                  <span className={`text-sm font-mono shrink-0 ${amountTone(posting.amount)}`}>
                    {formatAmount(posting.amount, posting.commodity)}
                  </span>
                </button>
                {onEditTransaction && (
                  <button onClick={() => onEditTransaction(posting.transactionIndex)}
                    className="px-3 min-h-[52px] min-w-[44px] text-blue-500 text-xs shrink-0">
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
