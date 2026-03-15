import { useState, useEffect, useMemo } from "react";
import * as api from "../api/commands";
import type { BalanceRow, RegisterRow } from "../api/types";

const ACCOUNT_TYPES = [
  { value: "", label: "All" },
  { value: "assets", label: "Assets" },
  { value: "liabilities", label: "Liabilities" },
  { value: "income", label: "Income" },
  { value: "expenses", label: "Expenses" },
  { value: "equity", label: "Equity" },
];

function formatAmount(amounts: { commodity: string; quantity: string }[]): string {
  return amounts
    .map((a) => {
      const q = parseFloat(a.quantity);
      if (a.commodity && a.commodity.length === 1) {
        return `${a.commodity}${q.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
      }
      return a.commodity
        ? `${q.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })} ${a.commodity}`
        : q.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
    })
    .join(", ");
}

export function AccountsPage() {
  const [allAccounts, setAllAccounts] = useState<BalanceRow[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selectedAccount, setSelectedAccount] = useState<string | null>(null);
  const [register, setRegister] = useState<RegisterRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState("");

  useEffect(() => {
    api.listAccountsWithBalances().then((data) => {
      setAllAccounts(data);
      setLoading(false);
      // Auto-expand top-level
      const topLevel = new Set(data.filter((a) => a.depth === 0).map((a) => a.account));
      setExpanded(topLevel);
    });
  }, []);

  // Filter accounts by type and search
  const filteredAccounts = useMemo(() => {
    let result = allAccounts;

    if (typeFilter) {
      result = result.filter((a) => a.account.startsWith(typeFilter));
    }

    if (search.trim()) {
      const q = search.toLowerCase();
      result = result.filter((a) => a.account.toLowerCase().includes(q));
    }

    return result;
  }, [allAccounts, typeFilter, search]);

  // Determine visible accounts based on expanded state
  const visibleAccounts = useMemo(() => {
    // When searching, show all matches flat (ignore expand state)
    if (search.trim()) {
      return filteredAccounts;
    }

    return filteredAccounts.filter((row) => {
      // Adjust depth based on type filter
      const effectiveRoot = typeFilter || "";
      const relativeAccount = effectiveRoot
        ? row.account.startsWith(effectiveRoot + ":")
          ? row.account
          : row.account === effectiveRoot
            ? row.account
            : ""
        : row.account;

      if (!relativeAccount) return false;

      const parts = row.account.split(":");
      // Top level (or first under type filter) is always visible
      if (typeFilter) {
        if (parts.length <= 1) return true;
        // Check if all ancestors are expanded
        for (let i = 1; i < parts.length; i++) {
          const ancestor = parts.slice(0, i).join(":");
          if (!expanded.has(ancestor)) return false;
        }
        return true;
      }

      if (row.depth === 0) return true;
      for (let i = 1; i < parts.length; i++) {
        const ancestor = parts.slice(0, i).join(":");
        if (!expanded.has(ancestor)) return false;
      }
      return true;
    });
  }, [filteredAccounts, expanded, typeFilter, search]);

  const hasChildren = (account: string) =>
    filteredAccounts.some((a) => a.account !== account && a.account.startsWith(account + ":"));

  const expandAll = () => {
    const all = new Set(filteredAccounts.map((a) => a.account));
    setExpanded(all);
  };

  const collapseAll = () => {
    // Keep only top-level expanded
    const topLevel = new Set(filteredAccounts.filter((a) => a.depth === 0).map((a) => a.account));
    setExpanded(topLevel);
  };

  const toggleExpand = (account: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(account)) {
        next.delete(account);
      } else {
        next.add(account);
      }
      return next;
    });
  };

  const handleAccountTap = async (account: string) => {
    setSelectedAccount(account);
    const data = await api.registerReport(account);
    setRegister(data);
  };

  // Register view for selected account
  if (selectedAccount) {
    return (
      <div className="flex flex-col h-full">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <button
            onClick={() => setSelectedAccount(null)}
            className="p-2 -ml-2 text-gray-600 dark:text-gray-300"
          >
            &larr;
          </button>
          <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100 truncate font-mono">
            {selectedAccount}
          </h2>
        </div>
        <div className="flex-1 overflow-auto">
          {register.length === 0 ? (
            <div className="flex items-center justify-center h-32 text-gray-500 dark:text-gray-400 text-sm">
              No postings
            </div>
          ) : (
            <div className="divide-y divide-gray-100 dark:divide-gray-800">
              {register.map((row, i) => (
                <div key={i} className="px-4 py-2.5">
                  <div className="flex justify-between items-center">
                    <div className="min-w-0 flex-1">
                      <div className="text-sm text-gray-900 dark:text-gray-100 truncate">
                        {row.description}
                      </div>
                      <div className="text-xs text-gray-500 dark:text-gray-400">{row.date}</div>
                    </div>
                    <div className="text-right ml-3 shrink-0">
                      <div className={`text-sm font-mono ${parseFloat(row.amount[0]?.quantity ?? "0") < 0 ? "text-red-500" : "text-green-500"}`}>
                        {formatAmount(row.amount)}
                      </div>
                      <div className="text-xs text-gray-400 font-mono">
                        {formatAmount(row.runningTotal)}
                      </div>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="px-4 py-3 border-b border-gray-200 dark:border-gray-700 space-y-2">
        <div className="flex items-center justify-between">
          <h1 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Accounts</h1>
          <div className="flex gap-2">
            <button onClick={expandAll} className="text-xs text-gray-500 dark:text-gray-400 active:text-gray-700 dark:active:text-gray-200">
              Expand
            </button>
            <span className="text-xs text-gray-300 dark:text-gray-600">|</span>
            <button onClick={collapseAll} className="text-xs text-gray-500 dark:text-gray-400 active:text-gray-700 dark:active:text-gray-200">
              Collapse
            </button>
          </div>
        </div>

        {/* Search */}
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search accounts..."
          className="w-full px-3 py-2 bg-gray-100 dark:bg-gray-800 rounded-lg text-sm text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:bg-white dark:focus:bg-gray-700"
        />

        {/* Type filter */}
        <div className="flex gap-1.5 overflow-x-auto pb-1 -mx-1 px-1">
          {ACCOUNT_TYPES.map((type) => (
            <button
              key={type.value}
              onClick={() => setTypeFilter(type.value)}
              className={`px-3 py-1.5 text-xs font-medium rounded-full whitespace-nowrap ${
                typeFilter === type.value
                  ? "bg-blue-600 text-white"
                  : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400 active:bg-gray-200 dark:active:bg-gray-700"
              }`}
            >
              {type.label}
            </button>
          ))}
        </div>
      </div>

      {/* Account tree */}
      <div className="flex-1 overflow-auto">
        {loading ? (
          <div className="flex items-center justify-center h-32 text-gray-500 dark:text-gray-400 text-sm">
            Loading...
          </div>
        ) : visibleAccounts.length === 0 ? (
          <div className="flex items-center justify-center h-32 text-gray-500 dark:text-gray-400 text-sm">
            No accounts found
          </div>
        ) : (
          <div className="divide-y divide-gray-50 dark:divide-gray-800">
            {visibleAccounts.map((row) => {
              const isExpanded = expanded.has(row.account);
              const canExpand = hasChildren(row.account);
              const shortName = search.trim()
                ? row.account
                : row.account.split(":").pop() ?? row.account;
              const isNegative = parseFloat(row.amounts[0]?.quantity ?? "0") < 0;
              const displayDepth = search.trim() ? 0 : row.depth;

              return (
                <div
                  key={row.account}
                  className="flex items-center px-4 py-2.5 min-h-[44px]"
                >
                  {/* Indent */}
                  <div style={{ width: displayDepth * 16 }} className="shrink-0" />

                  {/* Expand/collapse */}
                  {canExpand && !search.trim() ? (
                    <button
                      onClick={() => toggleExpand(row.account)}
                      className="w-6 h-6 flex items-center justify-center text-gray-400 shrink-0"
                    >
                      {isExpanded ? "\u25BE" : "\u25B8"}
                    </button>
                  ) : (
                    <div className="w-6 shrink-0" />
                  )}

                  {/* Account name */}
                  <button
                    onClick={() => handleAccountTap(row.account)}
                    className="flex-1 text-left text-sm text-gray-900 dark:text-gray-100 truncate"
                  >
                    {shortName}
                  </button>

                  {/* Balance */}
                  <span
                    className={`text-sm font-mono shrink-0 ml-2 ${
                      isNegative ? "text-red-500" : "text-green-500"
                    }`}
                  >
                    {formatAmount(row.amounts)}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
