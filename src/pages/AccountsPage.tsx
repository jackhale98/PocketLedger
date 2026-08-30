import { useState, useEffect, useMemo, useCallback, useRef } from "react";
import * as api from "../api/commands";
import { useNavStore } from "../store/navStore";
import { useJournalStore } from "../store/journalStore";
import { useAccountsViewStore } from "../store/accountsViewStore";
import { amountTone, decimalSign, formatAmount } from "../utils/format";
import { isRevealedBy, parentAccounts, toggleCollapsed } from "../utils/tree";
import type { ValuationMode } from "../api/commands";
import type { BalanceRow } from "../api/types";

const ACCOUNT_TYPES = [
  { value: "", label: "All" },
  { value: "assets", label: "Assets" },
  { value: "liabilities", label: "Liabilities" },
  { value: "income", label: "Income" },
  { value: "expenses", label: "Expenses" },
  { value: "equity", label: "Equity" },
];

/** Case-insensitive check if account matches a type filter */
function matchesType(account: string, typeFilter: string): boolean {
  if (!typeFilter) return true;
  const lower = account.toLowerCase();
  return lower === typeFilter || lower.startsWith(typeFilter + ":");
}

/** Every ancestor name of every account in `accounts`. */
function ancestorsOf(accounts: Iterable<string>): Set<string> {
  const out = new Set<string>();
  for (const account of accounts) {
    const parts = account.split(":");
    for (let i = 1; i < parts.length; i++) out.add(parts.slice(0, i).join(":"));
  }
  return out;
}

const chip = (active: boolean) =>
  `px-3 min-h-[44px] text-xs font-medium rounded-full whitespace-nowrap ${
    active
      ? "bg-blue-600 text-white"
      : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400 active:bg-gray-200 dark:active:bg-gray-700"
  }`;

export function AccountsPage() {
  const [allAccounts, setAllAccounts] = useState<BalanceRow[]>([]);
  const [loading, setLoading] = useState(true);
  // Filters and the opened tree survive a drill-down into the register.
  const expanded = useAccountsViewStore((s) => s.expanded);
  const setExpanded = useAccountsViewStore((s) => s.setExpanded);
  const initializeExpanded = useAccountsViewStore((s) => s.initializeExpanded);
  const search = useAccountsViewStore((s) => s.search);
  const setSearch = useAccountsViewStore((s) => s.setSearch);
  const typeFilter = useAccountsViewStore((s) => s.typeFilter);
  const setTypeFilter = useAccountsViewStore((s) => s.setTypeFilter);
  const valueCurrency = useAccountsViewStore((s) => s.valueCurrency);
  const valuation = useAccountsViewStore((s) => s.valuation);
  const setValuation = useAccountsViewStore((s) => s.setValuation);
  const hideZero = useAccountsViewStore((s) => s.hideZero);
  const setHideZero = useAccountsViewStore((s) => s.setHideZero);
  const setValueCurrency = useAccountsViewStore((s) => s.setValueCurrency);
  // Balances are stale once the journal changes under us.
  const loadGeneration = useJournalStore((s) => s.loadGeneration);
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
          ? api.listAccountsWithBalances({ targetCommodity: valueCurrency }, valuation)
          : api.listAccountsWithBalances(),
        api.listCommodities(),
      ]);
      if (seq !== loadSeq.current) return;
      setAllAccounts(data);
      setCommodities(comms);
      // Auto-expand top-level accounts
      const topLevel = new Set(data.filter((a: BalanceRow) => a.depth === 0).map((a: BalanceRow) => a.account));
      initializeExpanded(topLevel);
    } catch (err) {
      if (seq !== loadSeq.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
    // loadGeneration is a trigger, not an input.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [valueCurrency, valuation, initializeExpanded, loadGeneration]);

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

    if (hideZero) {
      // Keep a parent whose descendants still have a balance, or hiding a
      // zeroed parent would orphan its children.
      const nonZero = result
        .filter((a) => a.amounts.some((m) => decimalSign(m.quantity) !== 0))
        .map((a) => a.account);
      const keep = new Set(nonZero);
      for (const anc of ancestorsOf(nonZero)) keep.add(anc);
      result = result.filter((a) => keep.has(a.account));
    }

    return result;
  }, [allAccounts, typeFilter, search, hideZero]);

  // Which rows have children, computed once per filtered set rather than by
  // scanning the whole list for every row rendered.
  const parents = useMemo(() => parentAccounts(filteredAccounts), [filteredAccounts]);

  // Determine visible accounts based on expanded state
  const visibleAccounts = useMemo(() => {
    if (search.trim()) {
      return filteredAccounts;
    }
    return filteredAccounts.filter((row) => {
      // Under a type filter the type root (e.g. "Assets") is always visible;
      // otherwise so is any depth-0 row. Below that every ancestor must be
      // expanded.
      if (typeFilter ? row.account.split(":").length <= 1 : row.depth === 0) return true;
      return isRevealedBy(expanded, row.account);
    });
  }, [filteredAccounts, expanded, typeFilter, search]);

  const expandAll = () => {
    setExpanded(new Set(filteredAccounts.map((a) => a.account)));
  };

  const collapseAll = () => {
    const topLevel = new Set(filteredAccounts.filter((a) => a.depth === 0).map((a) => a.account));
    setExpanded(topLevel);
  };

  const navigate = useNavStore((s) => s.navigate);

  const toggleExpand = (account: string) => {
    setExpanded(toggleCollapsed(expanded, account));
  };

  const handleAccountTap = (account: string) => {
    navigate("reports", { kind: "register", account }, { tab: "accounts" });
  };

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="px-4 py-2 border-b border-gray-200 dark:border-gray-700 space-y-2">
        <div className="flex items-center justify-between">
          <h1 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Accounts</h1>
          <div className="flex items-center -mr-2">
            <button onClick={expandAll} className="text-xs px-2 min-h-[44px] text-gray-500 dark:text-gray-400 active:text-gray-700 dark:active:text-gray-200">
              Expand
            </button>
            <span className="text-xs text-gray-300 dark:text-gray-600" aria-hidden="true">|</span>
            <button onClick={collapseAll} className="text-xs px-2 min-h-[44px] text-gray-500 dark:text-gray-400 active:text-gray-700 dark:active:text-gray-200">
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
          aria-label="Search accounts"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          enterKeyHint="search"
          className="w-full px-3 py-2 min-h-[44px] bg-gray-100 dark:bg-gray-800 rounded-lg text-sm text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:bg-white dark:focus:bg-gray-700"
        />

        {/* Type filter */}
        <div className="flex gap-1.5 overflow-x-auto -mx-1 px-1" role="group" aria-label="Account type">
          {ACCOUNT_TYPES.map((type) => (
            <button
              key={type.value}
              onClick={() => setTypeFilter(type.value)}
              aria-pressed={typeFilter === type.value}
              className={chip(typeFilter === type.value)}
            >
              {type.label}
            </button>
          ))}
          <button
            onClick={() => setHideZero(!hideZero)}
            aria-pressed={hideZero}
            className={chip(hideZero)}
          >
            Hide zero
          </button>
        </div>

        {/* Value in currency */}
        {commodities.length > 1 && (
          <div className="flex items-center gap-2">
            <span className="text-xs text-gray-500 dark:text-gray-400 shrink-0">Value in</span>
            <select
              value={valueCurrency}
              onChange={(e) => setValueCurrency(e.target.value)}
              aria-label="Value in currency"
              className="flex-1 px-2 min-h-[44px] bg-gray-100 dark:bg-gray-800 rounded-lg text-xs text-gray-900 dark:text-gray-100 border-none focus:outline-none focus:ring-2 focus:ring-blue-500"
            >
              <option value="">Original</option>
              {commodities.map((c) => (
                <option key={c} value={c}>{c}</option>
              ))}
            </select>
            {/* Only meaningful once amounts are being converted: what a
                holding cost, or the gain since, needs a currency to say it in. */}
            {valueCurrency && (
              <select
                value={valuation}
                onChange={(e) => setValuation(e.target.value as ValuationMode)}
                aria-label="Valuation"
                className="shrink-0 min-w-0 px-2 min-h-[44px] bg-gray-100 dark:bg-gray-800 rounded-lg text-xs text-gray-900 dark:text-gray-100 border-none focus:outline-none focus:ring-2 focus:ring-blue-500"
              >
                <option value="market">Value</option>
                <option value="cost">Cost</option>
                <option value="gain">Gain</option>
              </select>
            )}
          </div>
        )}
      </div>

      {/* Account tree */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden">
        {error && (
          <div className="mx-4 mt-3 flex items-center justify-between text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">
            <span className="min-w-0 break-words">{error}</span>
            <button onClick={loadAccounts} className="text-xs text-red-500 ml-2 shrink-0 underline min-h-[44px] px-2">Retry</button>
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
          <div className="divide-y divide-gray-50 dark:divide-gray-800" role="tree">
            {visibleAccounts.map((row) => {
              const isExpanded = expanded.has(row.account);
              const canExpand = parents.has(row.account);
              const shortName = search.trim()
                ? row.account
                : row.account.split(":").pop() ?? row.account;
              const displayDepth = search.trim() ? 0 : row.depth;

              return (
                <div
                  key={row.account}
                  role="treeitem"
                  aria-expanded={canExpand && !search.trim() ? isExpanded : undefined}
                  className="flex items-center pl-4 pr-4 min-h-[44px]"
                >
                  {/* Indent */}
                  <div style={{ width: displayDepth * 16 }} className="shrink-0" />

                  {/* Expand/collapse */}
                  {canExpand && !search.trim() ? (
                    <button
                      onClick={() => toggleExpand(row.account)}
                      aria-label={isExpanded ? `Collapse ${shortName}` : `Expand ${shortName}`}
                      className="w-8 -ml-2 min-h-[44px] flex items-center justify-center text-gray-400 shrink-0"
                    >
                      {isExpanded ? "▾" : "▸"}
                    </button>
                  ) : (
                    <div className="w-6 shrink-0" />
                  )}

                  {/* Account name */}
                  <button
                    onClick={() => handleAccountTap(row.account)}
                    title={row.account}
                    className="flex-1 min-w-0 min-h-[44px] text-left text-sm text-gray-900 dark:text-gray-100 truncate"
                  >
                    {shortName}
                  </button>

                  {/* Balance. A parent holding several commodities gets one
                      line each, the way hledger prints it. */}
                  <div className="shrink-0 ml-2 text-right max-w-[55%] flex flex-col items-end py-1">
                    {row.amounts.map((amount, i) => (
                      <span
                        key={`${amount.commodity}-${i}`}
                        className={`text-sm font-mono truncate max-w-full ${amountTone(amount.quantity)}`}
                      >
                        {formatAmount(amount.quantity, amount.commodity)}
                      </span>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
