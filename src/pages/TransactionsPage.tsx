import { useState, useMemo } from "react";
import { useJournalStore } from "../store/journalStore";
import { useSettingsStore } from "../store/settingsStore";
import { TransactionList } from "../components/transactions/TransactionList";
import { TransactionDetail } from "../components/transactions/TransactionDetail";
import { TransactionForm } from "../components/transactions/TransactionForm";
import { DateFilter } from "../components/common/DateFilter";
import * as api from "../api/commands";

export function TransactionsPage() {
  const { transactions, addTransaction, refresh } = useJournalStore();
  const { defaultCurrency } = useSettingsStore();
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [editIndex, setEditIndex] = useState<number | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [dateFrom, setDateFrom] = useState("");
  const [dateTo, setDateTo] = useState("");
  const [sortNewestFirst, setSortNewestFirst] = useState(true);
  const [showFilters, setShowFilters] = useState(false);

  const filteredTransactions = useMemo(() => {
    let result = [...transactions];

    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      result = result.filter(
        (txn) =>
          txn.description.toLowerCase().includes(q) ||
          txn.postings.some((p) => p.account.toLowerCase().includes(q))
      );
    }

    if (dateFrom) result = result.filter((txn) => txn.date >= dateFrom);
    if (dateTo) result = result.filter((txn) => txn.date <= dateTo);
    if (sortNewestFirst) result.reverse();

    return result;
  }, [transactions, searchQuery, dateFrom, dateTo, sortNewestFirst]);

  const selectedTransaction =
    selectedIndex !== null
      ? transactions.find((t) => t.index === selectedIndex) ?? null
      : null;

  const editTransaction =
    editIndex !== null
      ? transactions.find((t) => t.index === editIndex) ?? null
      : null;

  if (showForm || editTransaction) {
    const prefill = editTransaction
      ? {
          date: editTransaction.date,
          status: editTransaction.status,
          description: editTransaction.description,
          comment: editTransaction.comment ?? "",
          postings: editTransaction.postings.map((p) => ({
            account: p.account,
            amount: p.amount ?? "",
            commodity: p.commodity ?? defaultCurrency,
            comment: p.comment ?? "",
          })),
        }
      : undefined;

    return (
      <TransactionForm
        defaultCurrency={defaultCurrency}
        prefill={prefill}
        title={editTransaction ? "Edit Transaction" : "New Transaction"}
        onSave={async (txn) => {
          if (editIndex !== null) {
            await api.updateTransaction(editIndex, txn);
            await refresh();
            setEditIndex(null);
          } else {
            await addTransaction(txn);
            setShowForm(false);
          }
        }}
        onCancel={() => { setShowForm(false); setEditIndex(null); }}
      />
    );
  }

  if (selectedTransaction) {
    return (
      <TransactionDetail
        transaction={selectedTransaction}
        onBack={() => setSelectedIndex(null)}
        onEdit={() => { setEditIndex(selectedIndex); setSelectedIndex(null); }}
        onDelete={async () => {
          await api.deleteTransaction(selectedIndex!);
          await refresh();
          setSelectedIndex(null);
        }}
      />
    );
  }

  const hasActiveFilters = dateFrom || dateTo;

  return (
    <div className="flex flex-col h-full relative">
      <div className="px-4 py-3 border-b border-gray-200 dark:border-gray-700 space-y-2">
        <div className="flex items-center justify-between">
          <h1 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Transactions</h1>
          <div className="flex gap-2">
            <button
              onClick={() => setSortNewestFirst(!sortNewestFirst)}
              className="text-xs font-medium px-2 py-1 rounded text-gray-500 dark:text-gray-400"
            >
              {sortNewestFirst ? "New \u2193" : "Old \u2191"}
            </button>
            <button
              onClick={() => setShowFilters(!showFilters)}
              className={`text-xs font-medium px-2 py-1 rounded ${
                hasActiveFilters
                  ? "bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400"
                  : "text-gray-500 dark:text-gray-400"
              }`}
            >
              {showFilters ? "Hide" : "Filter"}
            </button>
          </div>
        </div>

        <input
          type="text"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          placeholder="Search transactions..."
          className="w-full px-3 py-2 bg-gray-100 dark:bg-gray-800 rounded-lg text-sm text-gray-900 dark:text-gray-100 placeholder-gray-500 dark:placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500"
        />

        {showFilters && (
          <DateFilter
            dateFrom={dateFrom}
            dateTo={dateTo}
            onChange={(from, to) => { setDateFrom(from); setDateTo(to); }}
          />
        )}

        {(searchQuery || hasActiveFilters) && (
          <div className="text-xs text-gray-500 dark:text-gray-400">
            {filteredTransactions.length} of {transactions.length} transactions
          </div>
        )}
      </div>

      <div className="flex-1 overflow-auto">
        <TransactionList transactions={filteredTransactions} onSelect={setSelectedIndex} />
      </div>

      <button
        onClick={() => setShowForm(true)}
        className="absolute bottom-4 right-4 w-14 h-14 bg-blue-600 text-white rounded-full shadow-lg flex items-center justify-center text-2xl font-light active:bg-blue-700"
        aria-label="Add transaction"
      >
        +
      </button>
    </div>
  );
}
