import { useState, useCallback } from "react";
import { format } from "date-fns";
import { Autocomplete } from "../common/Autocomplete";
import * as api from "../../api/commands";
import type { NewPosting } from "../../api/types";

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
  onSave: (txn: {
    date: string;
    status: string;
    description: string;
    comment: string | null;
    postings: NewPosting[];
  }) => Promise<void>;
  onCancel: () => void;
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
}: TransactionFormProps) {
  const [date, setDate] = useState(prefill?.date ?? format(new Date(), "yyyy-MM-dd"));
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
      const val = parseFloat(p.amount);
      if (isNaN(val)) return "";
      total += val;
    }

    if (total === 0) return "";
    return (-total).toFixed(2);
  };

  const handleSubmit = async () => {
    setError(null);

    if (!description.trim()) {
      setError("Description is required");
      return;
    }

    const filledPostings = postings.filter((p) => p.account.trim() !== "");
    if (filledPostings.length < 2) {
      setError("At least 2 postings are required");
      return;
    }

    // Build postings for the API
    const apiPostings: NewPosting[] = filledPostings.map((p) => ({
      account: p.account.trim(),
      amount: p.amount.trim() || null,
      commodity: p.amount.trim() ? p.commodity || null : null,
      comment: p.comment.trim() || null,
    }));

    // Verify at most one posting has no amount
    const emptyAmountCount = apiPostings.filter((p) => p.amount === null).length;
    if (emptyAmountCount > 1) {
      setError("At most one posting can have an inferred amount");
      return;
    }

    setSaving(true);
    try {
      await onSave({
        date,
        status,
        description: description.trim(),
        comment: comment.trim() || null,
        postings: apiPostings,
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setSaving(false);
    }
  };

  const balancingAmount = getBalancingAmount();

  return (
    <div className="flex flex-col h-full bg-white dark:bg-gray-900">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        <button
          onClick={onCancel}
          className="text-gray-600 text-sm font-medium min-w-[60px]"
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
      <div className="flex-1 overflow-auto p-4 space-y-4">
        {error && (
          <div className="text-sm text-red-600 bg-red-50 px-3 py-2 rounded-lg">
            {error}
          </div>
        )}

        {/* Date */}
        <div>
          <label className="block text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">
            Date
          </label>
          <input
            type="date"
            value={date}
            onChange={(e) => setDate(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
        </div>

        {/* Status */}
        <div>
          <label className="block text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">
            Status
          </label>
          <div className="flex gap-2">
            {STATUS_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setStatus(opt.value)}
                className={`flex-1 py-2 text-sm rounded-lg border ${
                  status === opt.value
                    ? "border-blue-500 bg-blue-50 text-blue-700 font-medium"
                    : "border-gray-300 text-gray-600 dark:text-gray-300"
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
          <label className="block text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">
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
          <label className="block text-xs font-medium text-gray-500 uppercase tracking-wide mb-1">
            Note
          </label>
          <input
            type="text"
            value={comment}
            onChange={(e) => setComment(e.target.value)}
            placeholder="Optional note or comment"
            className="w-full px-3 py-2 border border-gray-300 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
        </div>

        {/* Postings */}
        <div>
          <div className="flex items-center justify-between mb-2">
            <label className="text-xs font-medium text-gray-500 uppercase tracking-wide">
              Postings
            </label>
            <button
              onClick={addPosting}
              className="text-xs text-blue-600 font-medium"
            >
              + Add Posting
            </button>
          </div>

          <div className="space-y-3">
            {postings.map((posting, index) => {
              const isLastEmpty =
                posting.amount.trim() === "" &&
                postings.filter((p) => p.amount.trim() === "").length === 1;

              return (
                <div
                  key={posting.id}
                  className="bg-gray-50 rounded-lg p-3 space-y-2"
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
                        className={`w-full px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono focus:outline-none focus:ring-2 focus:ring-blue-500 ${
                          isLastEmpty && balancingAmount
                            ? "placeholder:text-gray-400 dark:text-gray-500"
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
                      className="w-16 px-2 py-2 border border-gray-300 rounded-lg text-sm text-center focus:outline-none focus:ring-2 focus:ring-blue-500"
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
                    className="w-full px-3 py-1.5 border border-gray-200 rounded text-xs text-gray-600 focus:outline-none focus:ring-2 focus:ring-blue-500"
                  />
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}
