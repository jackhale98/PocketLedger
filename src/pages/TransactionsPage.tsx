import { useState, useMemo, useEffect, useRef, useCallback } from "react";
import { useJournalStore } from "../store/journalStore";
import { useSettingsStore } from "../store/settingsStore";
import { TransactionList } from "../components/transactions/TransactionList";
import { TransactionDetail } from "../components/transactions/TransactionDetail";
import { TransactionForm } from "../components/transactions/TransactionForm";
import { DateFilter } from "../components/common/DateFilter";
import * as api from "../api/commands";
import type { TransactionSummary } from "../api/types";

/** Detects hledger query syntax (acct:food, amt:>100, not:rent, ...) */
const QUERY_PREFIX_RE =
  /(^|\s)(acct|desc|payee|code|note|tag|cur|amt|status|date|real|depth|not):/;

const PAGE_SIZE = 100;

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
  const [advancedSearch, setAdvancedSearch] = useState(false);
  const [searchResults, setSearchResults] = useState<TransactionSummary[] | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [searching, setSearching] = useState(false);
  const [hintDismissed, setHintDismissed] = useState(false);
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);
  const searchSeq = useRef(0);
  const observerRef = useRef<IntersectionObserver | null>(null);

  const isBackendQuery =
    (advancedSearch || QUERY_PREFIX_RE.test(searchQuery)) && !!searchQuery.trim();

  // Backend query search, debounced (~250ms), with a seq guard against races.
  useEffect(() => {
    const seq = ++searchSeq.current;
    if (!isBackendQuery) {
      setSearchResults(null);
      setSearchError(null);
      setSearching(false);
      return;
    }
    setSearching(true);
    const timer = setTimeout(async () => {
      try {
        const results = await api.searchTransactions(searchQuery);
        if (seq !== searchSeq.current) return;
        setSearchResults(results);
        setSearchError(null);
      } catch (err) {
        if (seq !== searchSeq.current) return;
        // Keep the previous results visible; just surface the error.
        setSearchError(err instanceof Error ? err.message : String(err));
      } finally {
        if (seq === searchSeq.current) setSearching(false);
      }
    }, 250);
    return () => clearTimeout(timer);
  }, [searchQuery, isBackendQuery, transactions]);

  const filteredTransactions = useMemo(() => {
    let result =
      isBackendQuery && searchResults !== null
        ? [...searchResults]
        : [...transactions];

    // Plain-text substring filter only when not in backend query mode.
    if (!isBackendQuery && searchQuery.trim()) {
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
  }, [transactions, searchResults, isBackendQuery, searchQuery, dateFrom, dateTo, sortNewestFirst]);

  // Reset the render window whenever the filtered set changes.
  useEffect(() => {
    setVisibleCount(PAGE_SIZE);
  }, [filteredTransactions]);

  // Sentinel at the list bottom grows the window when it scrolls into view.
  const sentinelRef = useCallback((node: HTMLDivElement | null) => {
    observerRef.current?.disconnect();
    observerRef.current = null;
    if (node) {
      const observer = new IntersectionObserver((entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setVisibleCount((c) => c + PAGE_SIZE);
        }
      });
      observer.observe(node);
      observerRef.current = observer;
    }
  }, []);

  useEffect(() => () => observerRef.current?.disconnect(), []);

  const visibleTransactions =
    filteredTransactions.length > visibleCount
      ? filteredTransactions.slice(0, visibleCount)
      : filteredTransactions;

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
        // Editing rewrites the transaction in whichever file owns it, so the
        // destination only needs choosing when creating one.
        chooseFile={editIndex === null}
        onSave={async (txn, fileIndex) => {
          if (editIndex !== null) {
            await api.updateTransaction(editIndex, txn);
            await refresh();
            setEditIndex(null);
          } else {
            await addTransaction(txn, fileIndex);
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
              onClick={() => setAdvancedSearch(!advancedSearch)}
              className={`text-xs font-medium px-2 py-1 rounded ${
                advancedSearch
                  ? "bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400"
                  : "text-gray-500 dark:text-gray-400"
              }`}
            >
              Query
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
          placeholder={
            advancedSearch
              ? "Query: acct:food amt:>100 not:rent..."
              : "Search transactions..."
          }
          className="w-full px-3 py-2 bg-gray-100 dark:bg-gray-800 rounded-lg text-sm text-gray-900 dark:text-gray-100 placeholder-gray-500 dark:placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500"
        />

        {searchError && (
          <div className="text-xs text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg break-words">
            {searchError}
          </div>
        )}

        {(advancedSearch || isBackendQuery) && !hintDismissed && (
          <div className="flex items-start justify-between gap-2 text-xs text-gray-500 dark:text-gray-400 bg-gray-50 dark:bg-gray-800/60 px-3 py-2 rounded-lg">
            <span className="min-w-0 break-words">
              Examples: <code className="font-mono">acct:food</code>{" "}
              <code className="font-mono">amt:&gt;100</code>{" "}
              <code className="font-mono">date:2026-01..</code>{" "}
              <code className="font-mono">cur:EUR</code>{" "}
              <code className="font-mono">status:*</code>{" "}
              <code className="font-mono">tag:name=value</code>{" "}
              <code className="font-mono">not:rent</code>
            </span>
            <button
              onClick={() => setHintDismissed(true)}
              className="shrink-0 text-gray-400 dark:text-gray-500"
              aria-label="Dismiss hint"
            >
              &times;
            </button>
          </div>
        )}

        {showFilters && (
          <DateFilter
            dateFrom={dateFrom}
            dateTo={dateTo}
            onChange={(from, to) => { setDateFrom(from); setDateTo(to); }}
          />
        )}

        {(searchQuery || hasActiveFilters) && (
          <div className="text-xs text-gray-500 dark:text-gray-400">
            {searching
              ? "Searching..."
              : `${filteredTransactions.length} of ${transactions.length} transactions`}
          </div>
        )}
      </div>

      <div className="flex-1 overflow-y-auto overflow-x-hidden">
        <TransactionList transactions={visibleTransactions} onSelect={setSelectedIndex} />
        {filteredTransactions.length > visibleCount && (
          <div
            ref={sentinelRef}
            className="py-3 text-center text-xs text-gray-400 dark:text-gray-500"
          >
            Showing {visibleTransactions.length} of {filteredTransactions.length}
          </div>
        )}
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
