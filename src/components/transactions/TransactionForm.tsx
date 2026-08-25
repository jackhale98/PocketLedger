import { useState, useCallback, useRef, useEffect } from "react";
import { format } from "date-fns";
import { Autocomplete } from "../common/Autocomplete";
import * as api from "../../api/commands";
import { normalizeAmountInput, exactDecimalSum, isDecimalZero } from "../../utils/amount";
import type { NewPosting, JournalFileInfo } from "../../api/types";

interface PrefillData {
  date: string;
  status: string;
  description: string;
  comment: string;
  postings: { account: string; amount: string; commodity: string; comment: string }[];
}

interface TransactionFormProps {
  defaultCurrency?: string;
  prefill?: PrefillData;
  title?: string;
  onSave: (
    txn: {
      date: string;
      status: string;
      description: string;
      comment: string | null;
      postings: NewPosting[];
    },
    /** Index into listJournalFiles(); undefined means the main file. */
    fileIndex?: number
  ) => Promise<void>;
  onCancel: () => void;
  /** Offer a destination file. Only meaningful when creating. */
  chooseFile?: boolean;
}

interface PostingRow {
  id: number;
  account: string;
  amount: string;
  commodity: string;
  comment: string;
}

const STATUS_OPTIONS = [
  { value: "Unmarked", label: "Unmarked", symbol: "" },
  { value: "Pending", label: "Pending", symbol: "!" },
  { value: "Cleared", label: "Cleared", symbol: "*" },
];

let nextId = 1;

export function TransactionForm({
  defaultCurrency = "$",
  prefill,
  title = "New Transaction",
  onSave,
  onCancel,
  chooseFile = false,
}: TransactionFormProps) {
  const [date, setDate] = useState(prefill?.date ?? format(new Date(), "yyyy-MM-dd"));
  const [files, setFiles] = useState<JournalFileInfo[]>([]);
  const [fileIndex, setFileIndex] = useState(0);

  useEffect(() => {
    if (!chooseFile) return;
    api
      .listJournalFiles()
      .then((list) => {
        setFiles(list);
        // Split journals are almost always organized by year, so default to
        // the file whose name carries the entry's year.
        const byYear = list.find((f) => f.name.includes(date.slice(0, 4)));
        setFileIndex(byYear ? byYear.index : 0);
      })
      .catch(() => setFiles([]));
    // Mount only: re-picking as the user edits the date would fight them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chooseFile]);
  const [status, setStatus] = useState(prefill?.status ?? "Unmarked");
  const [description, setDescription] = useState(prefill?.description ?? "");
  const [comment, setComment] = useState(prefill?.comment ?? "");
  const [postings, setPostings] = useState<PostingRow[]>(() => {
    if (prefill?.postings && prefill.postings.length > 0) {
      return prefill.postings.map((p) => ({
        id: nextId++,
        account: p.account,
        amount: p.amount,
        commodity: p.commodity || defaultCurrency,
        comment: p.comment,
      }));
    }
    return [
      { id: nextId++, account: "", amount: "", commodity: defaultCurrency, comment: "" },
      { id: nextId++, account: "", amount: "", commodity: defaultCurrency, comment: "" },
    ];
  });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const submittingRef = useRef(false);

  const suggestAccounts = useCallback(
    (prefix: string) => api.suggestAccounts(prefix),
    []
  );

  const suggestDescriptions = useCallback(
    (prefix: string) => api.suggestDescriptions(prefix),
    []
  );

  const updatePosting = (id: number, field: keyof PostingRow, value: string) => {
    setPostings((prev) =>
      prev.map((p) => (p.id === id ? { ...p, [field]: value } : p))
    );
  };

  const addPosting = () => {
    setPostings((prev) => [
      ...prev,
      { id: nextId++, account: "", amount: "", commodity: defaultCurrency, comment: "" },
    ]);
  };

  const removePosting = (id: number) => {
    if (postings.length <= 2) return;
    setPostings((prev) => prev.filter((p) => p.id !== id));
  };

  // Calculate the balancing amount for the last empty posting
  const getBalancingAmount = (): string => {
    const filledPostings = postings.filter((p) => p.amount.trim() !== "");
    const emptyPostings = postings.filter((p) => p.amount.trim() === "");

    if (emptyPostings.length !== 1) return "";

    // Only calculate if all filled postings use the same commodity
    const commodities = new Set(filledPostings.map((p) => p.commodity));
    if (commodities.size > 1) return "";

    let total = 0;
    for (const p of filledPostings) {
      const normalized = normalizeAmountInput(p.amount);
      if (normalized === null) return "";
      total += parseFloat(normalized);
    }

    if (total === 0) return "";
    return (-total).toFixed(2);
  };

  const handleSubmit = async () => {
    if (submittingRef.current) return;
    setError(null);

    if (!description.trim()) {
      setError("Description is required");
      return;
    }
    if (/[\n;]/.test(description)) {
      setError("Description cannot contain \";\" or line breaks");
      return;
    }
    if (/\n/.test(comment)) {
      setError("Note cannot contain line breaks");
      return;
    }

    // A posting with an amount must also have an account
    const orphanIndex = postings.findIndex(
      (p) => p.amount.trim() !== "" && p.account.trim() === ""
    );
    if (orphanIndex !== -1) {
      setError(`Posting ${orphanIndex + 1} has an amount but no account`);
      return;
    }

    const filledPostings = postings.filter((p) => p.account.trim() !== "");
    if (filledPostings.length < 2) {
      setError("At least 2 postings are required");
      return;
    }

    for (let i = 0; i < filledPostings.length; i++) {
      const p = filledPostings[i];
      if (/\n/.test(p.account) || /\n/.test(p.comment)) {
        setError(`Posting ${i + 1} cannot contain line breaks`);
        return;
      }
    }

    // Normalize amounts (accepts comma-decimal input, e.g. "1,5" -> "1.5")
    const normalizedAmounts: (string | null)[] = [];
    for (let i = 0; i < filledPostings.length; i++) {
      const raw = filledPostings[i].amount.trim();
      if (raw === "") {
        normalizedAmounts.push(null);
        continue;
      }
      const normalized = normalizeAmountInput(raw);
      if (normalized === null) {
        setError(`"${raw}" is not a valid amount (posting ${i + 1})`);
        return;
      }
      normalizedAmounts.push(normalized);
    }

    // Verify at most one posting has no amount
    const emptyAmountCount = normalizedAmounts.filter((a) => a === null).length;
    if (emptyAmountCount > 1) {
      setError("At most one posting can have an inferred amount");
      return;
    }

    // When every posting has an explicit amount in the same commodity,
    // they must sum to exactly zero (exact decimal math, no float rounding).
    if (emptyAmountCount === 0) {
      const commodities = new Set(filledPostings.map((p) => p.commodity));
      if (commodities.size === 1) {
        const sum = exactDecimalSum(normalizedAmounts as string[]);
        if (sum !== null && !isDecimalZero(sum)) {
          const commodity = filledPostings[0].commodity;
          setError(`Postings don't balance: off by ${sum} ${commodity}. Adjust an amount or leave one empty to infer it.`);
          return;
        }
      }
    }

    // Build postings for the API (dot-decimal amounts)
    const apiPostings: NewPosting[] = filledPostings.map((p, i) => ({
      account: p.account.trim(),
      amount: normalizedAmounts[i],
      commodity: normalizedAmounts[i] !== null ? p.commodity || null : null,
      comment: p.comment.trim() || null,
    }));

    submittingRef.current = true;
    setSaving(true);
    try {
      await onSave(
        {
          date,
          status,
          description: description.trim(),
          comment: comment.trim() || null,
          postings: apiPostings,
        },
        chooseFile ? fileIndex : undefined
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setSaving(false);
    } finally {
      submittingRef.current = false;
    }
  };

  const balancingAmount = getBalancingAmount();

  return (
    <div className="flex flex-col h-full bg-white dark:bg-gray-900 overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        <button
          onClick={onCancel}
          className="text-gray-600 dark:text-gray-300 text-sm font-medium min-w-[60px]"
        >
          Cancel
        </button>
        <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100">{title}</h2>
        <button
          onClick={handleSubmit}
          disabled={saving}
          className="text-blue-600 text-sm font-semibold min-w-[60px] text-right disabled:opacity-50"
        >
          {saving ? "Saving..." : "Save"}
        </button>
      </div>

      {/* Form */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 space-y-4">
        {error && (
          <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">
            {error}
          </div>
        )}

        {/* Date */}
        <div className="min-w-0">
          <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">
            Date
          </label>
          <input
            type="date"
            value={date}
            onChange={(e) => setDate(e.target.value)}
            className="w-full min-w-0 max-w-full box-border px-3 py-2 border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
        </div>

        {chooseFile && files.length > 1 && (
          <div className="min-w-0">
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">
              Add to file
            </label>
            <select
              value={fileIndex}
              onChange={(e) => setFileIndex(Number(e.target.value))}
              className="w-full min-w-0 truncate px-3 py-2 border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm"
            >
              {files.map((f) => (
                <option key={f.index} value={f.index}>
                  {f.name}{f.isMain ? " (main)" : ""}
                </option>
              ))}
            </select>
          </div>
        )}

        {/* Status */}
        <div>
          <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">
            Status
          </label>
          <div className="flex gap-2">
            {STATUS_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setStatus(opt.value)}
                className={`flex-1 py-2 text-sm rounded-lg border ${
                  status === opt.value
                    ? "border-blue-500 bg-blue-50 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400 font-medium"
                    : "border-gray-300 dark:border-gray-600 text-gray-600 dark:text-gray-300"
                }`}
              >
                {opt.symbol ? `${opt.symbol} ` : ""}
                {opt.label}
              </button>
            ))}
          </div>
        </div>

        {/* Description */}
        <div>
          <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">
            Description
          </label>
          <Autocomplete
            value={description}
            onChange={setDescription}
            onSuggest={suggestDescriptions}
            placeholder="Payee or description"
          />
        </div>

        {/* Note / Comment */}
        <div>
          <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-1">
            Note
          </label>
          <input
            type="text"
            value={comment}
            onChange={(e) => setComment(e.target.value)}
            placeholder="Optional note or comment"
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
        </div>

        {/* Postings */}
        <div>
          <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide mb-2">
            Postings
          </label>

          <div className="space-y-3">
            {postings.map((posting, index) => {
              const isLastEmpty =
                posting.amount.trim() === "" &&
                postings.filter((p) => p.amount.trim() === "").length === 1;

              return (
                <div
                  key={posting.id}
                  className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 space-y-2 overflow-hidden"
                >
                  <div className="flex items-center justify-between">
                    <span className="text-xs text-gray-400 dark:text-gray-500">
                      Posting {index + 1}
                    </span>
                    {postings.length > 2 && (
                      <button
                        onClick={() => removePosting(posting.id)}
                        className="text-xs text-red-500"
                      >
                        Remove
                      </button>
                    )}
                  </div>

                  {/* Account */}
                  <Autocomplete
                    value={posting.account}
                    onChange={(v) => updatePosting(posting.id, "account", v)}
                    onSuggest={suggestAccounts}
                    placeholder="Account name"
                  />

                  {/* Amount + Commodity */}
                  <div className="flex gap-2">
                    <div className="flex-1 relative">
                      <input
                        type="text"
                        inputMode="decimal"
                        value={posting.amount}
                        onChange={(e) =>
                          updatePosting(posting.id, "amount", e.target.value)
                        }
                        placeholder={
                          isLastEmpty && balancingAmount
                            ? balancingAmount
                            : "Amount (empty to infer)"
                        }
                        className={`w-full px-3 py-2 border border-gray-300 dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100 rounded-lg text-sm font-mono focus:outline-none focus:ring-2 focus:ring-blue-500 ${
                          isLastEmpty && balancingAmount
                            ? "placeholder:text-gray-400 dark:placeholder:text-gray-500"
                            : ""
                        }`}
                      />
                    </div>
                    <input
                      type="text"
                      value={posting.commodity}
                      onChange={(e) =>
                        updatePosting(posting.id, "commodity", e.target.value)
                      }
                      placeholder="$"
                      className="w-16 px-2 py-2 border border-gray-300 dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100 rounded-lg text-sm text-center focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>

                  {/* Posting comment */}
                  <input
                    type="text"
                    value={posting.comment}
                    onChange={(e) =>
                      updatePosting(posting.id, "comment", e.target.value)
                    }
                    placeholder="Posting note (optional)"
                    className="w-full px-3 py-1.5 border border-gray-200 dark:border-gray-600 dark:bg-gray-700 dark:text-gray-300 rounded text-xs text-gray-600 focus:outline-none focus:ring-2 focus:ring-blue-500"
                  />
                </div>
              );
            })}
          </div>

          <button
            onClick={addPosting}
            className="mt-3 w-full py-2.5 border border-dashed border-gray-300 dark:border-gray-600 rounded-lg text-sm font-medium text-blue-600 dark:text-blue-400 active:bg-blue-50 dark:active:bg-gray-800 min-h-[44px]"
          >
            + Add Posting
          </button>
        </div>
      </div>
    </div>
  );
}
