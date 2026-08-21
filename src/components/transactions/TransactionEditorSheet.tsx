import { useEffect, useState } from "react";
import * as api from "../../api/commands";
import type { TransactionSummary } from "../../api/types";
import { TransactionDetail } from "./TransactionDetail";
import { TransactionForm } from "./TransactionForm";

/** View/edit/delete one transaction by index, self-contained so any report
 *  that can name a transaction (the register, say) gets the same flow the
 *  Transactions tab uses. */
export function TransactionEditorSheet({
  index,
  defaultCurrency,
  onClose,
  onChanged,
}: {
  index: number;
  defaultCurrency: string;
  onClose: () => void;
  /** Called after a successful save or delete, so the caller can reload. */
  onChanged: () => void;
}) {
  const [txn, setTxn] = useState<TransactionSummary | null>(null);
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api
      .getTransaction(index)
      .then((t) => {
        if (live) setTxn(t);
      })
      .catch((err) => {
        if (live) setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      live = false;
    };
  }, [index]);

  if (error) {
    return (
      <div className="flex flex-col h-full">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <button onClick={onClose} className="p-2 -ml-2 text-gray-600 dark:text-gray-300">
            &larr;
          </button>
          <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">
            Transaction
          </h2>
        </div>
        <div className="p-4">
          <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg break-words">
            {error}
          </div>
        </div>
      </div>
    );
  }

  if (!txn) {
    return <div className="text-sm text-gray-500 text-center py-8">Loading...</div>;
  }

  if (editing) {
    return (
      <TransactionForm
        defaultCurrency={defaultCurrency}
        title="Edit Transaction"
        prefill={{
          date: txn.date,
          status: txn.status,
          description: txn.description,
          comment: txn.comment ?? "",
          postings: txn.postings.map((p) => ({
            account: p.account,
            amount: p.amount ?? "",
            commodity: p.commodity ?? defaultCurrency,
            comment: p.comment ?? "",
          })),
        }}
        onSave={async (updated) => {
          await api.updateTransaction(index, updated);
          onChanged();
          onClose();
        }}
        onCancel={() => setEditing(false)}
      />
    );
  }

  return (
    <TransactionDetail
      transaction={txn}
      onBack={onClose}
      onEdit={() => setEditing(true)}
      onDelete={async () => {
        await api.deleteTransaction(index);
        onChanged();
        onClose();
      }}
    />
  );
}
