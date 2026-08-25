import { useState, useEffect, useMemo, useCallback, useRef } from "react";
import * as api from "../api/commands";
import { useNavStore } from "../store/navStore";
import type { BalanceRow } from "../api/types";

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
      const isSymbol = a.commodity && a.commodity.length === 1
        && "$\u20AC\u00A3\u00A5\u20B9\u20BD\u20BF".includes(a.commodity);
      if (isSymbol) {
        return `${a.commodity}${q.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
      }
      const rawParts = a.quantity.split(".");
      const decimals = rawParts.length > 1 ? rawParts[1].replace(/0+$/, "").length : 0;
      const formatted = q.toLocaleString(undefined, {
        minimumFractionDigits: decimals,
        maximumFractionDigits: Math.max(decimals, 2),
      });
      return a.commodity ? `${formatted} ${a.commodity}` : formatted;
    })
    .join(", ");
}


/** Case-insensitive check if account matches a type filter */
function matchesType(account: string, typeFilter: string): boolean {
  if (!typeFilter) return true;
  const lower = account.toLowerCase();
  return lower === typeFilter || lower.startsWith(typeFilter + ":");
}

export function AccountsPage() {
  const [allAccounts, setAllAccounts] = useState<BalanceRow[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState("");
  const [valueCurrency, setValueCurrency] = useState<string>("");
  const [commodities, setCommodities] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const loadSeq = useRef(0);

  const loadAccounts = useCallback(async () => {
    const seq = ++loadSeq.current;
    setLoading(true);
    setError(null);
    try {
      const [data, comms] = await Promise.all([
        valueCurrency
          ? api.listAccountsWithBalances({ targetCommodity: valueCurrency })
          : api.listAccountsWithBalances(),
        api.listCommodities(),
      ]);
      if (seq !== loadSeq.current) return;
      setAllAccounts(data);
      setCommodities(comms);
      // Auto-expand top-level accounts
      const topLevel = new Set(data.filter((a: BalanceRow) => a.depth === 0).map((a: BalanceRow) => a.account));
      setExpanded(topLevel);
    } catch (err) {
      if (seq !== loadSeq.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
  }, [valueCurrency]);

  useEffect(() => { loadAccounts(); }, [loadAccounts]);

  // Filter accounts by type and search (case-insensitive)
  const filteredAccounts = useMemo(() => {
    let result = allAccounts;

    if (typeFilter) {
      result = result.filter((a) => matchesType(a.account, typeFilter));
    }

    if (search.trim()) {
      const q = search.toLowerCase();
      result = result.filter((a) => a.account.toLowerCase().includes(q));
    }

    return result;
  }, [allAccounts, typeFilter, search]);

  // Determine visible accounts based on expanded state
  const visibleAccounts = useMemo(() => {
    if (search.trim()) {
      return filteredAccounts;
    }

    return filteredAccounts.filter((row) => {
      const parts = row.account.split(":");

      if (typeFilter) {
        // Under a type filter, the type root (e.g. "Assets") is always visible
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
    const topLevel = new Set(filteredAccounts.filter((a) => a.depth === 0).map((a) => a.account));
    setExpanded(topLevel);
  };

  const navigate = useNavStore((s) => s.navigate);

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

  const handleAccountTap = (account: string) => {
    navigate("reports", { kind: "register", account });
  };

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

        {/* Value in currency */}
        {commodities.length > 1 && (
          <div className="flex items-center gap-2">
            <span className="text-xs text-gray-500 dark:text-gray-400 shrink-0">Value in</span>
            <select
              value={valueCurrency}
              onChange={(e) => setValueCurrency(e.target.value)}
              className="flex-1 px-2 py-1.5 bg-gray-100 dark:bg-gray-800 rounded-lg text-xs text-gray-900 dark:text-gray-100 border-none focus:outline-none focus:ring-2 focus:ring-blue-500"
            >
              <option value="">Original</option>
              {commodities.map((c) => (
                <option key={c} value={c}>{c}</option>
              ))}
            </select>
          </div>
        )}
      </div>

      {/* Account tree */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden">
        {error && (
          <div className="mx-4 mt-3 flex items-center justify-between text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">
            <span className="min-w-0 break-words">{error}</span>
            <button onClick={loadAccounts} className="text-xs text-red-500 ml-2 shrink-0 underline">Retry</button>
          </div>
        )}
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
              const isLeaf = !canExpand;
              const isMultiCommodity = row.amounts.length > 1;
              // In valued mode (single currency), always show totals including parents.
              // In original mode, hide parent accounts with many commodities.
              const showAmount = !!valueCurrency || isLeaf || row.amounts.length <= 2;
              const isNegative = !isMultiCommodity && parseFloat(row.amounts[0]?.quantity ?? "0") < 0;
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
                    title={row.account}
                    className="flex-1 text-left text-sm text-gray-900 dark:text-gray-100 truncate"
                  >
                    {shortName}
                  </button>

                  {/* Balance */}
                  {showAmount && (
                    <span
                      className={`text-sm font-mono shrink-0 ml-2 text-right max-w-[45%] truncate ${
                        isMultiCommodity
                          ? "text-gray-700 dark:text-gray-300"
                          : isNegative ? "text-red-500" : "text-green-500"
                      }`}
                    >
                      {formatAmount(row.amounts)}
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
