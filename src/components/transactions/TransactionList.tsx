
import type { TransactionSummary } from "../../api/types";
import { AmountDisplay } from "../common/AmountDisplay";
import { StatusBadge } from "../common/StatusBadge";

interface TransactionListProps {
  transactions: TransactionSummary[];
  onSelect: (index: number) => void;
}

export function TransactionList({
  transactions,
  onSelect,
}: TransactionListProps) {
  if (transactions.length === 0) {
    return (
      <div className="flex items-center justify-center h-64 text-gray-500 dark:text-gray-400">
        No transactions
      </div>
    );
  }

  return (
    <div className="divide-y divide-gray-100 dark:divide-gray-800">
      {transactions.map((txn) => (
        <button
          key={txn.index}
          className="w-full px-4 py-3 flex items-center gap-3 active:bg-gray-50 dark:active:bg-gray-800 text-left min-h-[56px]"
          onClick={() => onSelect(txn.index)}
        >
          <StatusBadge status={txn.status} />
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium text-gray-900 dark:text-gray-100 truncate">
              {txn.description}
            </div>
            <div className="text-xs text-gray-500 dark:text-gray-400">{txn.date}</div>
          </div>
          <div className="text-right shrink-0">
            {txn.postings.length > 0 && (
              <AmountDisplay
                amount={txn.postings[0].amount}
                commodity={txn.postings[0].commodity}
                className="text-sm"
              />
            )}
          </div>
        </button>
      ))}
    </div>
  );
}
