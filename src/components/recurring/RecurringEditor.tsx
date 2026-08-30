import { useState, useCallback, useEffect } from "react";
import { Autocomplete } from "../common/Autocomplete";
import { SignToggle } from "../common/SignToggle";
import { useSettingsStore } from "../../store/settingsStore";
import * as api from "../../api/commands";
import { normalizeAmountInput } from "../../utils/amount";
import { formatAmount } from "../../utils/format";
import type { JournalFileInfo, ForecastRule, SaveForecastPosting } from "../../api/types";
import { useBackHandler } from "../../store/backStore";

const PERIOD_PRESETS = [
  { value: "monthly", label: "Monthly" },
  { value: "weekly", label: "Weekly" },
  { value: "quarterly", label: "Quarterly" },
  { value: "yearly", label: "Yearly" },
  { value: "every 2 weeks", label: "Every 2 weeks" },
];

interface RuleLine {
  account: string;
  amount: string;
  commodity: string;
}

export function RecurringEditor({ onDone }: { onDone: () => void }) {
  const { defaultCurrency } = useSettingsStore();
  const [rules, setRules] = useState<ForecastRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState(false);

  const [period, setPeriod] = useState("monthly");
  const [description, setDescription] = useState("");
  const [lines, setLines] = useState<RuleLine[]>([
    { account: "", amount: "", commodity: defaultCurrency },
    { account: "", amount: "", commodity: defaultCurrency },
  ]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Source line of the rule being edited; null = creating a new one. */
  const [editingLine, setEditingLine] = useState<number | null>(null);
  const [files, setFiles] = useState<JournalFileInfo[]>([]);
  const [fileIndex, setFileIndex] = useState(0);
  const [deletingLine, setDeletingLine] = useState<number | null>(null);
  /** Rule awaiting a second tap to confirm deletion. */
  const [confirmDeleteLine, setConfirmDeleteLine] = useState<number | null>(null);

  const loadRules = useCallback(async () => {
    try {
      const r = await api.getForecastRules();
      setRules(r);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    loadRules().finally(() => setLoading(false));
  }, [loadRules]);

  useEffect(() => {
    api.listJournalFiles().then(setFiles).catch(() => setFiles([]));
  }, []);

  const loadFromExisting = (rule: ForecastRule, asCopy = false) => {
    setPeriod(rule.period);
    setDescription(rule.description);
    setLines(
      rule.postings.map((p) => ({
        account: p.account,
        amount: p.amount ?? "",
        commodity: p.commodity || defaultCurrency,
      }))
    );
    setError(null);
    // editingLine null means "save as new", which is what duplicating wants.
    setEditingLine(asCopy ? null : rule.line);
    setEditing(true);
  };

  const startNew = () => {
    setPeriod("monthly");
    setDescription("");
    setLines([
      { account: "", amount: "", commodity: defaultCurrency },
      { account: "", amount: "", commodity: defaultCurrency },
    ]);
    setError(null);
    setEditingLine(null);
    setEditing(true);
  };

  const handleDelete = async (rule: ForecastRule) => {
    setConfirmDeleteLine(null);
    try {
      setDeletingLine(rule.line);
      await api.deleteForecastRule(rule.line, rule.fileIndex);
      await loadRules();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setDeletingLine(null);
    }
  };

  const updateLine = (index: number, field: keyof RuleLine, value: string) => {
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

  /** Validate postings and normalize amounts to dot-decimal; sets error and
   *  returns null on bad input. */
  const normalizedPostings = (): SaveForecastPosting[] | null => {
    if (!period.trim()) {
      setError("Enter a period, e.g. monthly");
      return null;
    }
    if (!description.trim()) {
      setError("Enter a description for the recurring transaction");
      return null;
    }
    const filled = lines.filter((l) => l.account.trim() || l.amount.trim());
    if (filled.length === 0) {
      setError("Add at least one posting with an account");
      return null;
    }
    const result: SaveForecastPosting[] = [];
    let elided = 0;
    for (const l of filled) {
      const account = l.account.trim();
      if (!account) {
        setError(`Posting with amount "${l.amount}" is missing its account`);
        return null;
      }
      if (!l.amount.trim()) {
        elided++;
        result.push({ account, amount: null, commodity: l.commodity.trim() || null });
        continue;
      }
      const amount = normalizeAmountInput(l.amount);
      if (amount === null) {
        setError(`Invalid amount "${l.amount}" for ${account}`);
        return null;
      }
      result.push({ account, amount, commodity: l.commodity.trim() || null });
    }
    if (elided > 1) {
      setError(
        "Leave at most one posting without an amount — hledger can only infer one balancing amount."
      );
      return null;
    }
    return result;
  };

  const handleSave = async () => {
    const postings = normalizedPostings();
    if (!postings) return;

    try {
      setSaving(true);
      await api.saveForecastRule(
        period.trim(),
        description.trim(),
        postings,
        editingLine,
        // Editing rewrites the rule wherever it already lives, so the choice
        // only applies to a new one.
        editingLine === null ? fileIndex : undefined
      );
      onDone();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  const fmtAmt = (amount: string | null, commodity: string) => {
    if (amount === null) return "auto";
    return formatAmount(amount, commodity);
  };

  // Mirrors the arrows below: back dismisses a pending confirmation, then
  // leaves the form, then the list.
  useBackHandler(true, () => {
    if (confirmDeleteLine !== null) setConfirmDeleteLine(null);
    else if (editing) setEditing(false);
    else onDone();
  });

  // Existing rules list view
  if (!editing) {
    return (
      <div className="flex flex-col h-full">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <button onClick={onDone} aria-label="Back" className="p-2 -ml-2 min-w-[44px] min-h-[44px] text-gray-600 dark:text-gray-300">
            &larr;
          </button>
          <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">Recurring Rules</h2>
        </div>

        <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 space-y-4">
          {error && (
            <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg break-words">
              {error}
            </div>
          )}

          <p className="text-xs text-gray-500 dark:text-gray-400">
            One list, two uses: hledger drives both budget goals and forecasts
            from the same recurring rules. Each rule here becomes a goal in the
            Budget report and a projected transaction in the Forecast report.
          </p>

          {loading ? (
            <div className="text-sm text-gray-500 text-center py-8">Loading...</div>
          ) : rules.length === 0 ? (
            <div className="text-center py-8 space-y-3">
              <div className="text-sm text-gray-500 dark:text-gray-400">No recurring rules yet</div>
              <p className="text-xs text-gray-400 dark:text-gray-500">
                Add rent, salary or subscriptions to set budget goals and project
                your balance forward
              </p>
            </div>
          ) : (
            <>
              <label className="text-sm font-medium text-gray-700 dark:text-gray-300 block">
                Existing Rules
              </label>
              {rules.map((rule, ri) => (
                <div
                  key={ri}
                  className={`rounded-lg p-3 space-y-2 ${
                    rule.error
                      ? "bg-amber-50 dark:bg-amber-900/20 border border-amber-300 dark:border-amber-700"
                      : "bg-gray-50 dark:bg-gray-800"
                  }`}
                >
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0">
                      <div className="text-sm font-medium text-gray-900 dark:text-gray-100 truncate" title={rule.description}>
                        {rule.description || <span className="italic text-gray-400">No description</span>}
                      </div>
                      <div className="text-xs text-gray-500 dark:text-gray-400 break-words">{rule.period}</div>
                    </div>
                    <div className="flex shrink-0 -my-2 -mr-2">
                      <button
                        onClick={() => loadFromExisting(rule)}
                        className="text-xs text-blue-600 dark:text-blue-400 font-medium min-h-[44px] min-w-[44px] px-2"
                      >
                        Edit
                      </button>
                      <button
                        onClick={() => loadFromExisting(rule, true)}
                        className="text-xs text-blue-600 dark:text-blue-400 font-medium min-h-[44px] min-w-[44px] px-2"
                      >
                        Copy
                      </button>
                      <button
                        onClick={() => setConfirmDeleteLine(confirmDeleteLine === rule.line ? null : rule.line)}
                        disabled={deletingLine === rule.line}
                        aria-expanded={confirmDeleteLine === rule.line}
                        className="text-xs text-red-600 dark:text-red-400 font-medium min-h-[44px] min-w-[44px] px-2 disabled:opacity-50"
                      >
                        {deletingLine === rule.line ? "Deleting..." : "Delete"}
                      </button>
                    </div>
                  </div>
                  {confirmDeleteLine === rule.line && (
                    <div className="space-y-2">
                      <p className="text-xs text-red-600 dark:text-red-400">
                        This removes the rule from your journal file.
                      </p>
                      <div className="flex gap-2">
                        <button
                          onClick={() => setConfirmDeleteLine(null)}
                          className="flex-1 min-h-[44px] text-sm font-medium text-gray-600 dark:text-gray-400 rounded-lg border border-gray-300 dark:border-gray-600"
                        >
                          Cancel
                        </button>
                        <button
                          onClick={() => handleDelete(rule)}
                          disabled={deletingLine === rule.line}
                          className="flex-1 min-h-[44px] text-sm font-medium text-white bg-red-600 rounded-lg active:bg-red-700 disabled:opacity-50"
                        >
                          Delete
                        </button>
                      </div>
                    </div>
                  )}
                  {rule.error && (
                    <div className="text-xs text-amber-800 dark:text-amber-300 break-words">
                      &#9888; Generates nothing: {rule.error}
                    </div>
                  )}
                  <div className="divide-y divide-gray-200 dark:divide-gray-700">
                    {rule.postings.map((p, pi) => (
                      <div key={pi} className="flex justify-between py-1.5">
                        <span className="text-xs text-gray-600 dark:text-gray-400 truncate" title={p.account}>
                          {p.account}
                        </span>
                        <span className={`text-xs font-mono shrink-0 ml-2 ${p.amount === null ? "text-gray-400 italic" : "text-gray-900 dark:text-gray-100"}`}>
                          {fmtAmt(p.amount, p.commodity || defaultCurrency)}
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
            className="w-full py-3 min-h-[48px] bg-blue-600 text-white rounded-lg text-sm font-medium"
          >
            Create Recurring Transaction
          </button>
        </div>
      </div>
    );
  }

  // Edit/create form view
  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        <button onClick={() => setEditing(false)} aria-label="Back" className="p-2 -ml-2 min-w-[44px] min-h-[44px] text-gray-600 dark:text-gray-300">
          &larr;
        </button>
        <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">
          {editingLine !== null ? "Edit Rule" : "New Rule"}
        </h2>
      </div>

      <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 space-y-4">
        {editingLine === null && files.length > 1 && (
          <div className="min-w-0">
            <label className="text-sm font-medium text-gray-700 dark:text-gray-300 block mb-2">
              Add to file
            </label>
            <select
              value={fileIndex}
              onChange={(e) => setFileIndex(Number(e.target.value))}
              className="w-full min-w-0 truncate min-h-[48px] px-3 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100"
            >
              {files.map((f) => (
                <option key={f.index} value={f.index}>
                  {f.name}{f.isMain ? " (main)" : ""}
                </option>
              ))}
            </select>
          </div>
        )}

        {/* Description */}
        <div>
          <label className="text-sm font-medium text-gray-700 dark:text-gray-300 block mb-2">
            Description
          </label>
          <input
            type="text"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="e.g. Rent"
            aria-label="Description"
            autoCapitalize="sentences"
            autoCorrect="off"
            spellCheck={false}
            enterKeyHint="next"
            className="w-full min-h-[48px] px-3 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100"
          />
        </div>

        {/* Period */}
        <div>
          <label className="text-sm font-medium text-gray-700 dark:text-gray-300 block mb-2">
            Repeats
          </label>
          <div className="flex gap-1.5 flex-wrap mb-2">
            {PERIOD_PRESETS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setPeriod(opt.value)}
                aria-pressed={period === opt.value}
                className={`py-2 px-3 min-h-[44px] text-xs font-medium rounded-lg ${
                  period === opt.value
                    ? "bg-blue-600 text-white"
                    : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400"
                }`}
              >
                {opt.label}
              </button>
            ))}
          </div>
          <input
            type="text"
            value={period}
            onChange={(e) => setPeriod(e.target.value)}
            placeholder="monthly from 2026-01"
            aria-label="Period expression"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            enterKeyHint="next"
            className="w-full min-h-[48px] px-3 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg text-sm font-mono text-gray-900 dark:text-gray-100"
          />
          <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">
            daily / weekly / monthly / quarterly / yearly, &ldquo;every N days|weeks|months|quarters|years&rdquo;,
            &ldquo;every 15th day of month&rdquo;, &ldquo;every friday&rdquo; &mdash; each optionally with
            &ldquo;from DATE&rdquo; and &ldquo;to DATE&rdquo;.
          </p>
        </div>

        {/* Postings */}
        <div className="space-y-3">
          <label className="text-sm font-medium text-gray-700 dark:text-gray-300 block">
            Postings
          </label>
          {lines.map((line, i) => (
            <div key={i} className="space-y-2 bg-gray-50 dark:bg-gray-800 rounded-lg p-3">
              <Autocomplete
                value={line.account}
                onChange={(v) => updateLine(i, "account", v)}
                onSuggest={suggestAccounts}
                placeholder="Account (e.g. expenses:rent)"
                aria-label={`Posting ${i + 1} account`}
                className="w-full"
              />
              <div className="flex gap-2 min-w-0">
                <input
                  type="text"
                  inputMode="decimal"
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                  enterKeyHint="next"
                  aria-label={`Posting ${i + 1} amount`}
                  value={line.amount}
                  onChange={(e) => updateLine(i, "amount", e.target.value)}
                  placeholder="Amount (blank = balance)"
                  className="flex-1 min-w-0 px-3 py-2 min-h-[44px] bg-white dark:bg-gray-700 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100"
                />
                <SignToggle value={line.amount} onChange={(v) => updateLine(i, "amount", v)} />
                <input
                  type="text"
                  value={line.commodity}
                  onChange={(e) => updateLine(i, "commodity", e.target.value)}
                  placeholder="$"
                  aria-label={`Posting ${i + 1} commodity`}
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                  className="w-14 shrink-0 px-1 py-2 min-h-[44px] text-center bg-white dark:bg-gray-700 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100 text-center"
                />
                {lines.length > 1 && (
                  <button
                    onClick={() => removeLine(i)}
                    aria-label={`Remove posting ${i + 1}`}
                    className="px-2 shrink-0 min-h-[44px] min-w-[44px] text-red-500 text-lg font-medium"
                  >
                    &times;
                  </button>
                )}
              </div>
            </div>
          ))}
          <button
            onClick={addLine}
            className="w-full py-2.5 min-h-[48px] border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-500 dark:text-gray-400 font-medium"
          >
            + Add Posting
          </button>
          <p className="text-xs text-gray-400 dark:text-gray-500">
            Leave one amount blank and hledger fills it in to balance the transaction.
          </p>
        </div>

        {/* Error */}
        {error && (
          <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg break-words">
            {error}
          </div>
        )}

        <button
          onClick={handleSave}
          disabled={saving}
          className="w-full py-3 min-h-[48px] bg-blue-600 text-white rounded-lg text-sm font-medium disabled:opacity-50"
        >
          {saving ? "Saving..." : editingLine !== null ? "Save Changes" : "Add to Journal"}
        </button>
      </div>
    </div>
  );
}
