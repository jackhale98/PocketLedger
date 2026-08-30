import { useState } from "react";
import type { TransactionSummary } from "../../api/types";
import { AmountDisplay } from "../common/AmountDisplay";
import { postingAmounts } from "./postingAmounts";
import { StatusBadge } from "../common/StatusBadge";
import { useBackHandler } from "../../store/backStore";

interface TransactionDetailProps {
  transaction: TransactionSummary;
  onBack: () => void;
  onEdit: () => void;
  onDelete: () => void | Promise<void>;
  /** Start a new transaction prefilled from this one. */
  onDuplicate?: () => void;
}

export function TransactionDetail({
  transaction,
  onBack,
  onEdit,
  onDelete,
  onDuplicate,
}: TransactionDetailProps) {
  // The delete confirmation is the innermost thing back should dismiss.
  const [showConfirmDelete, setShowConfirmDelete] = useState(false);
  useBackHandler(true, () => {
    if (showConfirmDelete) setShowConfirmDelete(false);
    else onBack();
  });
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const handleDelete = async () => {
    if (deleting) return;
    setDeleting(true);
    setDeleteError(null);
    try {
      await onDelete();
    } catch (err) {
      setDeleteError(err instanceof Error ? err.message : String(err));
    } finally {
      setDeleting(false);
    }
  };

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        <button
          onClick={onBack}
          className="p-2 -ml-2 min-w-[44px] min-h-[44px] text-gray-600 dark:text-gray-400"
          aria-label="Back"
        >
          &larr;
        </button>
        <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100 truncate flex-1">
          {transaction.description}
        </h2>
        {onDuplicate && (
          <button
            onClick={onDuplicate}
            className="text-blue-600 text-sm font-medium px-2 min-h-[44px]"
          >
            Duplicate
          </button>
        )}
        <button
          onClick={onEdit}
          className="text-blue-600 text-sm font-medium px-2 min-h-[44px]"
        >
          Edit
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 space-y-4 selectable">
        {/* Metadata */}
        <div className="flex items-center gap-3">
          <StatusBadge status={transaction.status} />
          <span className="text-sm text-gray-600 dark:text-gray-400">{transaction.date}</span>
        </div>

        {/* Transaction comment */}
        {transaction.comment && (
          <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg px-3 py-2">
            <span className="text-xs font-medium text-amber-700 dark:text-amber-400 uppercase tracking-wide">Note</span>
            <p className="text-sm text-amber-900 dark:text-amber-200 mt-0.5">{transaction.comment}</p>
          </div>
        )}

        {/* Postings */}
        <div className="space-y-2">
          <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 uppercase tracking-wide">
            Postings
          </h3>
          <div className="bg-gray-50 dark:bg-gray-800 rounded-lg divide-y divide-gray-200 dark:divide-gray-700">
            {transaction.postings.map((posting, i) => (
              <div key={i} className="px-4 py-3">
                <div className="flex justify-between items-start gap-2">
                  <span className="text-sm text-gray-900 dark:text-gray-100 font-mono break-all min-w-0">
                    {posting.account}
                  </span>
                  {/* One line per commodity when the posting carries several. */}
                  <span className="flex flex-col items-end shrink-0">
                    {postingAmounts(posting).map((a, ai) => (
                      <AmountDisplay
                        key={ai}
                        amount={a.quantity}
                        commodity={a.commodity}
                        className="text-sm"
                      />
                    ))}
                  </span>
                </div>
                {posting.comment && (
                  <p className="text-xs text-gray-500 dark:text-gray-400 mt-1 italic">
                    {posting.comment}
                  </p>
                )}
              </div>
            ))}
          </div>
        </div>

        {/* Delete */}
        <div className="pt-4">
          {!showConfirmDelete ? (
            <button
              onClick={() => setShowConfirmDelete(true)}
              className="w-full py-3 text-red-600 text-sm font-medium rounded-lg border border-red-200 dark:border-red-800 active:bg-red-50 dark:active:bg-red-900/20"
            >
              Delete Transaction
            </button>
          ) : (
            <div className="space-y-2">
              <p className="text-sm text-red-600 text-center">
                This will permanently remove the transaction from your journal file.
              </p>
              {deleteError && (
                <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">
                  {deleteError}
                </div>
              )}
              <div className="flex gap-2">
                <button
                  onClick={() => setShowConfirmDelete(false)}
                  className="flex-1 py-3 text-sm font-medium text-gray-600 dark:text-gray-400 rounded-lg border border-gray-300 dark:border-gray-600"
                >
                  Cancel
                </button>
                <button
                  onClick={handleDelete}
                  disabled={deleting}
                  className="flex-1 py-3 text-sm font-medium text-white bg-red-600 rounded-lg active:bg-red-700 disabled:opacity-50"
                >
                  {deleting ? "Deleting..." : "Delete"}
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
