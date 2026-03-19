import { useState, useCallback, useEffect } from "react";
import { Autocomplete } from "../common/Autocomplete";
import { useSettingsStore } from "../../store/settingsStore";
import * as api from "../../api/commands";
import type { BudgetInfo } from "../../api/types";

const PERIOD_OPTIONS = [
  { value: "monthly", label: "Monthly" },
  { value: "quarterly", label: "Quarterly" },
  { value: "yearly", label: "Yearly" },
  { value: "weekly", label: "Weekly" },
];

interface BudgetLine {
  account: string;
  amount: string;
  commodity: string;
}

export function BudgetEditor({ onDone }: { onDone: () => void }) {
  const { defaultCurrency } = useSettingsStore();
  const [existingBudgets, setExistingBudgets] = useState<BudgetInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState(false);

  const [period, setPeriod] = useState("monthly");
  const [lines, setLines] = useState<BudgetLine[]>([
    { account: "", amount: "", commodity: defaultCurrency },
  ]);
  const [preview, setPreview] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.getBudgets().then((b) => {
      setExistingBudgets(b);
      setLoading(false);
    });
  }, []);

  const loadFromExisting = (budget: BudgetInfo) => {
    setPeriod(budget.period);
    setLines(
      budget.entries.map((e) => ({
        account: e.account,
        amount: e.amount,
        commodity: e.commodity || defaultCurrency,
      }))
    );
    setPreview(null);
    setError(null);
    setEditing(true);
  };

  const startNew = () => {
    setPeriod("monthly");
    setLines([{ account: "", amount: "", commodity: defaultCurrency }]);
    setPreview(null);
    setError(null);
    setEditing(true);
  };

  const updateLine = (index: number, field: keyof BudgetLine, value: string) => {
    setLines((prev) => {
      const next = [...prev];
      next[index] = { ...next[index], [field]: value };
      return next;
    });
  };

  const addLine = () => {
    setLines((prev) => [...prev, { account: "", amount: "", commodity: defaultCurrency }]);
  };

  const removeLine = (index: number) => {
    if (lines.length <= 1) return;
    setLines((prev) => prev.filter((_, i) => i !== index));
  };

  const suggestAccounts = useCallback(async (prefix: string) => {
    return api.suggestAccounts(prefix);
  }, []);

  const generatePreview = () => {
    const validLines = lines.filter((l) => l.account && l.amount);
    if (validLines.length === 0) {
      setError("Add at least one budget entry with account and amount");
      return;
    }

    let text = `~ ${period}\n`;
    for (const line of validLines) {
      const acct = line.account.padEnd(36);
      const sym = line.commodity;
      const isSymbol = sym.length === 1 && "$\u20AC\u00A3\u00A5\u20B9\u20BD\u20BF\u20A9\u20AB\u20B4\u20B8\u20BA\u20A6\u20AD".includes(sym);
      const formatted = isSymbol
        ? `${sym}${parseFloat(line.amount).toFixed(2)}`
        : `${parseFloat(line.amount).toFixed(2)} ${sym}`;
      text += `    ${acct}${formatted}\n`;
    }

    setPreview(text);
    setError(null);
  };

  const handleSave = async () => {
    const validLines = lines.filter((l) => l.account && l.amount);
    if (validLines.length === 0) {
      setError("Add at least one budget entry with account and amount");
      return;
    }

    try {
      setSaving(true);
      await api.saveBudget(
        validLines.map((l) => ({
          account: l.account,
          amount: l.amount,
          commodity: l.commodity,
        })),
        period
      );
      onDone();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  const fmtAmt = (amount: string, commodity: string) => {
    const q = parseFloat(amount);
    const isSymbol = commodity.length === 1 && "$\u20AC\u00A3\u00A5\u20B9\u20BD\u20BF".includes(commodity);
    const qs = q.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
    return isSymbol ? `${commodity}${qs}` : `${qs} ${commodity}`;
  };

  // Existing budgets list view
  if (!editing) {
    return (
      <div className="flex flex-col h-full">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <button onClick={onDone} className="p-2 -ml-2 text-gray-600 dark:text-gray-300">
            &larr;
          </button>
          <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">Manage Budget</h2>
        </div>

        <div className="flex-1 overflow-auto p-4 space-y-4">
          {loading ? (
            <div className="text-sm text-gray-500 text-center py-8">Loading...</div>
          ) : existingBudgets.length === 0 ? (
            <div className="text-center py-8 space-y-3">
              <div className="text-sm text-gray-500 dark:text-gray-400">No budgets defined</div>
              <p className="text-xs text-gray-400 dark:text-gray-500">
                Create a budget to set spending targets for your accounts
              </p>
            </div>
          ) : (
            <>
              <label className="text-sm font-medium text-gray-700 dark:text-gray-300 block">
                Existing Budgets
              </label>
              {existingBudgets.map((budget, bi) => (
                <div key={bi} className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 space-y-2">
                  <div className="flex items-center justify-between">
                    <span className="text-sm font-medium text-gray-900 dark:text-gray-100 capitalize">
                      {budget.period}
                    </span>
                    <div className="flex gap-2">
                      <button
                        onClick={() => loadFromExisting(budget)}
                        className="text-xs text-blue-600 dark:text-blue-400 font-medium"
                      >
                        Copy &amp; Edit
                      </button>
                    </div>
                  </div>
                  <div className="divide-y divide-gray-200 dark:divide-gray-700">
                    {budget.entries.map((entry, ei) => (
                      <div key={ei} className="flex justify-between py-1.5">
                        <span className="text-xs text-gray-600 dark:text-gray-400 truncate">
                          {entry.account}
                        </span>
                        <span className="text-xs font-mono text-gray-900 dark:text-gray-100 shrink-0 ml-2">
                          {fmtAmt(entry.amount, entry.commodity || defaultCurrency)}
                        </span>
                      </div>
                    ))}
                  </div>
                </div>
              ))}
            </>
          )}

          <button
            onClick={startNew}
            className="w-full py-3 bg-blue-600 text-white rounded-lg text-sm font-medium"
          >
            Create New Budget
          </button>
        </div>
      </div>
    );
  }

  // Edit/create form view
  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        <button onClick={() => setEditing(false)} className="p-2 -ml-2 text-gray-600 dark:text-gray-300">
          &larr;
        </button>
        <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">
          {existingBudgets.length > 0 ? "New Budget" : "Create Budget"}
        </h2>
      </div>

      <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 space-y-4">
        {/* Period selector */}
        <div>
          <label className="text-sm font-medium text-gray-700 dark:text-gray-300 block mb-2">
            Budget Period
          </label>
          <div className="flex gap-1.5">
            {PERIOD_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setPeriod(opt.value)}
                className={`flex-1 py-2 text-xs font-medium rounded-lg ${
                  period === opt.value
                    ? "bg-blue-600 text-white"
                    : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400"
                }`}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </div>

        {/* Budget entries */}
        <div className="space-y-3">
          <label className="text-sm font-medium text-gray-700 dark:text-gray-300 block">
            Budget Entries
          </label>
          {lines.map((line, i) => (
            <div key={i} className="space-y-2 bg-gray-50 dark:bg-gray-800 rounded-lg p-3">
              <Autocomplete
                value={line.account}
                onChange={(v) => updateLine(i, "account", v)}
                onSuggest={suggestAccounts}
                placeholder="Account (e.g. expenses:food)"
                className="w-full"
              />
              <div className="flex gap-2">
                <input
                  type="text"
                  inputMode="decimal"
                  value={line.amount}
                  onChange={(e) => updateLine(i, "amount", e.target.value)}
                  placeholder="Amount"
                  className="flex-1 px-3 py-2 bg-white dark:bg-gray-700 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100"
                />
                <input
                  type="text"
                  value={line.commodity}
                  onChange={(e) => updateLine(i, "commodity", e.target.value)}
                  placeholder="$"
                  className="w-16 px-3 py-2 bg-white dark:bg-gray-700 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100 text-center"
                />
                {lines.length > 1 && (
                  <button
                    onClick={() => removeLine(i)}
                    className="px-3 py-2 text-red-500 text-sm font-medium"
                  >
                    Remove
                  </button>
                )}
              </div>
            </div>
          ))}
          <button
            onClick={addLine}
            className="w-full py-2.5 border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-500 dark:text-gray-400 font-medium"
          >
            + Add Entry
          </button>
        </div>

        {/* Error */}
        {error && (
          <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">
            {error}
          </div>
        )}

        {/* Preview */}
        {preview && (
          <div>
            <label className="text-sm font-medium text-gray-700 dark:text-gray-300 block mb-2">
              Preview
            </label>
            <pre className="bg-gray-100 dark:bg-gray-800 rounded-lg p-3 text-xs font-mono text-gray-800 dark:text-gray-200 overflow-x-auto">
              {preview}
            </pre>
          </div>
        )}

        {/* Actions */}
        <div className="flex gap-2">
          <button
            onClick={generatePreview}
            className="flex-1 py-3 bg-gray-100 dark:bg-gray-800 rounded-lg text-sm font-medium text-gray-700 dark:text-gray-300"
          >
            Preview
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="flex-1 py-3 bg-blue-600 text-white rounded-lg text-sm font-medium disabled:opacity-50"
          >
            {saving ? "Saving..." : "Add to Journal"}
          </button>
        </div>
      </div>
    </div>
  );
}
