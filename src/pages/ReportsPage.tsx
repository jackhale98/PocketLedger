import { useState, useEffect, useCallback, useRef } from "react";
import {
  LineChart, Line, BarChart, Bar, PieChart, Pie, Cell,
  XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer,
  ReferenceLine, Legend,
} from "recharts";
import * as api from "../api/commands";
import { useSettingsStore } from "../store/settingsStore";
import { useJournalStore } from "../store/journalStore";
import { useNavStore, type ReportTab } from "../store/navStore";
import { DateFilter } from "../components/common/DateFilter";
import { TransactionEditorSheet } from "../components/transactions/TransactionEditorSheet";
import type {
  TimeSeriesPoint, IncomeExpensePoint, PieSlice,
  FinancialStatement, BalanceRow, RegisterRow, ReportParams,
  BudgetRow, BudgetSummaryPoint, ForecastRule, InactiveBudget,
  AmountEntry, BalanceInterval, BalanceAccumulationMode, PeriodicBalanceReport,
  ForecastProjection, TransactionSummary,
} from "../api/types";

type DrillView = "balance-sheet" | "income-statement" | "cash-flow" | null;

const COLORS = ["#3b82f6","#ef4444","#22c55e","#f59e0b","#8b5cf6","#ec4899","#06b6d4","#84cc16","#f97316","#6366f1"];

function fmtAmt(amounts: { commodity: string; quantity: string }[]): string {
  return amounts.map((a) => {
    const q = parseFloat(a.quantity);
    const qs = q.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
    return a.commodity && a.commodity.length === 1 ? `${a.commodity}${qs}` : a.commodity ? `${qs} ${a.commodity}` : qs;
  }).join(", ");
}

function fmtBudgetAmt(value: string, commodity: string): string {
  const q = parseFloat(value);
  const qs = q.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  if (commodity.length === 1 && "$\u20AC\u00A3\u00A5\u20B9\u20BD\u20BF".includes(commodity)) {
    return `${commodity}${qs}`;
  }
  return commodity ? `${qs} ${commodity}` : qs;
}

function StatementView({ statement, subtitle, onBack }: { statement: FinancialStatement; subtitle?: string | null; onBack: () => void }) {
  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        <button onClick={onBack} className="p-2 -ml-2 text-gray-600 dark:text-gray-300">&larr;</button>
        <div className="min-w-0">
          <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100 truncate">{statement.title}</h2>
          {subtitle && <div className="text-xs text-gray-500 dark:text-gray-400">{subtitle}</div>}
        </div>
      </div>
      <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 space-y-4">
        {statement.sections.map((section, si) => (
          <div key={si}>
            <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 uppercase tracking-wide mb-2">{section.title}</h3>
            {section.rows.length > 0 ? (
              <div className="bg-gray-50 dark:bg-gray-800 rounded-lg divide-y divide-gray-200 dark:divide-gray-700">
                {section.rows.map((row, ri) => (
                  <div key={ri} className="px-3 py-2 flex justify-between" style={{ paddingLeft: `${12 + row.depth * 16}px` }}>
                    <span className="text-sm text-gray-800 dark:text-gray-200 truncate" title={row.account}>{row.account.split(":").pop()}</span>
                    <span className={`text-sm font-mono shrink-0 ml-2 ${parseFloat(row.amounts[0]?.quantity ?? "0") < 0 ? "text-red-500" : "text-green-500"}`}>{fmtAmt(row.amounts)}</span>
                  </div>
                ))}
              </div>
            ) : <div className="text-sm text-gray-400 italic">No data</div>}
            <div className="flex justify-between px-3 py-2 font-semibold text-sm text-gray-900 dark:text-gray-100">
              <span>Total</span><span className="font-mono">{fmtAmt(section.total)}</span>
            </div>
          </div>
        ))}
        <div className="border-t-2 border-gray-300 dark:border-gray-600 pt-2 flex justify-between font-bold text-sm text-gray-900 dark:text-gray-100">
          <span>Net</span><span className="font-mono">{fmtAmt(statement.net)}</span>
        </div>
      </div>
    </div>
  );
}

function RegisterView({ accountList, account, onAccountChange, dateFrom, dateTo, currency, onChanged }: { accountList: string[]; account: string; onAccountChange: (a: string) => void; dateFrom: string; dateTo: string; currency: string; onChanged: () => void }) {
  const [editIndex, setEditIndex] = useState<number | null>(null);
  const [rows, setRows] = useState<RegisterRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [newestFirst, setNewestFirst] = useState(true);
  const loadSeq = useRef(0);

  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    if (!account) { setRows([]); setError(null); setLoading(false); return; }
    setLoading(true);
    setError(null);
    try {
      const data = await api.registerReport(account, { dateFrom: dateFrom || null, dateTo: dateTo || null, targetCommodity: currency });
      if (seq !== loadSeq.current) return;
      setRows(data);
    } catch (err) {
      if (seq !== loadSeq.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
  }, [account, dateFrom, dateTo, currency]);

  useEffect(() => { load(); }, [load]);

  const displayRows = newestFirst ? [...rows].reverse() : rows;

  if (editIndex !== null) {
    return (
      <TransactionEditorSheet
        index={editIndex}
        defaultCurrency={currency}
        onClose={() => setEditIndex(null)}
        onChanged={() => { onChanged(); load(); }}
      />
    );
  }

  return (
    <div className="space-y-3 min-w-0">
      <div className="flex gap-2 items-stretch">
        <select value={account} onChange={(e) => onAccountChange(e.target.value)}
          className="flex-1 min-w-0 truncate px-3 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100">
          <option value="">Select an account...</option>
          {account && !accountList.includes(account) && <option value={account}>{account}</option>}
          {accountList.map((n) => <option key={n} value={n}>{n}</option>)}
        </select>
        <button
          onClick={() => setNewestFirst(!newestFirst)}
          title={newestFirst ? "Newest first" : "Oldest first"}
          className="px-3 py-2 bg-gray-100 dark:bg-gray-800 rounded-lg text-xs font-medium text-gray-600 dark:text-gray-400 shrink-0 whitespace-nowrap"
        >
          {newestFirst ? "New \u2193" : "Old \u2191"}
        </button>
      </div>
      {error && <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">{error}</div>}
      {loading && <div className="text-sm text-gray-500 text-center py-4">Loading...</div>}
      {!loading && !error && account && rows.length === 0 && <div className="text-sm text-gray-500 text-center py-4">No postings</div>}
      {displayRows.length > 0 && (
        <div className="divide-y divide-gray-100 dark:divide-gray-800 min-w-0">
          {displayRows.map((row, i) => {
            const editable = row.transactionIndex !== null;
            const body = (
              <>
                <div className="min-w-0 flex-1">
                  <div className="text-sm text-gray-900 dark:text-gray-100 truncate flex items-center gap-1.5" title={row.description}>
                    {row.generated && (
                      <span className="shrink-0 text-[10px] font-semibold uppercase tracking-wide px-1.5 py-0.5 rounded bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-400">
                        proj
                      </span>
                    )}
                    <span className="truncate">{row.description}</span>
                  </div>
                  <div className="text-xs text-gray-500 dark:text-gray-400">{row.date}</div>
                </div>
                <div className="text-right ml-3 shrink-0">
                  <div className={`text-sm font-mono ${parseFloat(row.amount[0]?.quantity ?? "0") < 0 ? "text-red-500" : "text-green-500"}`}>{fmtAmt(row.amount)}</div>
                  <div className="text-xs text-gray-400 font-mono">{fmtAmt(row.runningTotal)}</div>
                </div>
              </>
            );
            // Projected rows have no journal entry behind them, so they stay
            // inert rather than opening an editor that can't save.
            return editable ? (
              <button
                key={i}
                onClick={() => setEditIndex(row.transactionIndex)}
                className="w-full py-2.5 flex justify-between items-center gap-2 min-w-0 text-left active:bg-gray-50 dark:active:bg-gray-800"
              >
                {body}
              </button>
            ) : (
              <div key={i} className="py-2.5 flex justify-between items-center gap-2 min-w-0">
                {body}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function fmtCompactEntry(a: AmountEntry): string {
  const q = parseFloat(a.quantity);
  const qs = q.toLocaleString(undefined, { maximumFractionDigits: 2 });
  return a.commodity && a.commodity.length === 1 ? `${a.commodity}${qs}` : a.commodity ? `${qs} ${a.commodity}` : qs;
}

function BalanceCell({ amounts, bold }: { amounts: AmountEntry[]; bold?: boolean }) {
  return (
    <td className={`px-2 py-1.5 text-right font-mono whitespace-nowrap align-top ${bold ? "font-semibold" : ""}`} style={{ fontVariantNumeric: "tabular-nums" }}>
      {amounts.length === 0 ? (
        <span className="text-gray-300 dark:text-gray-600">&middot;</span>
      ) : (
        amounts.map((a, i) => (
          <div key={i} className={parseFloat(a.quantity) < 0 ? "text-red-500" : "text-gray-800 dark:text-gray-200"}>
            {fmtCompactEntry(a)}
          </div>
        ))
      )}
    </td>
  );
}

const TABLE_INTERVALS: [BalanceInterval, string][] = [
  ["weekly", "W"], ["monthly", "M"], ["quarterly", "Q"], ["yearly", "Y"],
];
const TABLE_DEPTHS: (number | null)[] = [null, 1, 2, 3, 4];

function TableView({ dateFrom, dateTo }: { dateFrom: string; dateTo: string }) {
  const [interval, setInterval_] = useState<BalanceInterval>("monthly");
  const [mode, setMode] = useState<BalanceAccumulationMode>("periodic");
  const [depth, setDepth] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [forecast, setForecast] = useState(false);
  const [report, setReport] = useState<PeriodicBalanceReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const loadSeq = useRef(0);

  useEffect(() => {
    const t = setTimeout(() => setDebouncedQuery(query), 300);
    return () => clearTimeout(t);
  }, [query]);

  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    setLoading(true);
    try {
      const data = await api.periodicBalance(interval, mode, depth, {
        dateFrom: dateFrom || null,
        dateTo: dateTo || null,
        query: debouncedQuery.trim() || null,
        forecast: forecast || null,
      });
      if (seq !== loadSeq.current) return;
      setReport(data);
      setError(null);
    } catch (err) {
      if (seq !== loadSeq.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
  }, [interval, mode, depth, debouncedQuery, forecast, dateFrom, dateTo]);

  useEffect(() => { load(); }, [load]);

  // Grand total (Total column footer): sum of row totals per commodity.
  const grandTotal: AmountEntry[] = (() => {
    if (!report) return [];
    const byCommodity = new Map<string, number>();
    for (const row of report.rows) {
      for (const a of row.total) {
        byCommodity.set(a.commodity, (byCommodity.get(a.commodity) ?? 0) + parseFloat(a.quantity));
      }
    }
    return [...byCommodity.entries()]
      .filter(([, q]) => q !== 0)
      .map(([commodity, q]) => ({ commodity, quantity: q.toString() }));
  })();

  const stickyCol = "sticky left-0 bg-white dark:bg-gray-900";

  return (
    <div className="space-y-3">
      {/* Interval + mode */}
      <div className="flex gap-2">
        <div className="flex rounded-lg overflow-hidden border border-gray-300 dark:border-gray-600">
          {TABLE_INTERVALS.map(([val, label]) => (
            <button key={val} onClick={() => setInterval_(val)}
              className={`px-3 py-2 text-xs font-medium ${val === interval ? "bg-blue-600 text-white" : "bg-white dark:bg-gray-800 text-gray-600 dark:text-gray-400"}`}>
              {label}
            </button>
          ))}
        </div>
        <select value={mode} onChange={(e) => setMode(e.target.value as BalanceAccumulationMode)}
          className="flex-1 px-3 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100">
          <option value="periodic">Periodic</option>
          <option value="cumulative">Cumulative</option>
          <option value="historical">Historical</option>
        </select>
      </div>

      {/* Depth + forecast */}
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-1">
          <span className="text-xs text-gray-500 dark:text-gray-400 mr-1">Depth</span>
          {TABLE_DEPTHS.map((d) => (
            <button key={d ?? "all"} onClick={() => setDepth(d)}
              className={`px-2.5 py-1.5 text-xs font-medium rounded ${d === depth ? "bg-blue-600 text-white" : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400"}`}>
              {d === null ? "All" : d}
            </button>
          ))}
        </div>
        <label className="flex items-center gap-1.5 text-xs text-gray-600 dark:text-gray-400 shrink-0">
          <input type="checkbox" checked={forecast} onChange={(e) => setForecast(e.target.checked)} className="accent-blue-600" />
          Forecast
        </label>
      </div>

      {/* Query */}
      <input
        type="text"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Query: acct:expenses cur:EUR not:rent..."
        className="w-full px-3 py-2 bg-gray-100 dark:bg-gray-800 rounded-lg text-sm text-gray-900 dark:text-gray-100 placeholder-gray-500 dark:placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500"
      />

      {error && (
        <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg break-words">{error}</div>
      )}

      {forecast && !error && (
        <p className="text-xs text-gray-400 dark:text-gray-500">includes forecast from periodic transactions</p>
      )}

      {loading ? (
        <div className="text-sm text-gray-500 text-center py-8">Loading...</div>
      ) : !error && report && report.rows.length === 0 ? (
        <div className="text-sm text-gray-500 text-center py-8">No data for this period</div>
      ) : !error && report ? (
        <div className="overflow-x-auto rounded-lg border border-gray-200 dark:border-gray-700">
          <table className="text-xs w-full min-w-max border-collapse">
            <thead>
              <tr className="border-b border-gray-200 dark:border-gray-700">
                <th className={`${stickyCol} px-2 py-1.5 text-left font-medium text-gray-500 dark:text-gray-400`}>Account</th>
                {report.periods.map((p) => (
                  <th key={p} className="px-2 py-1.5 text-right font-medium text-gray-500 dark:text-gray-400 whitespace-nowrap">{p}</th>
                ))}
                <th className="px-2 py-1.5 text-right font-medium text-gray-500 dark:text-gray-400">Total</th>
              </tr>
            </thead>
            <tbody>
              {report.rows.map((row) => (
                <tr key={row.account} className="border-b border-gray-100 dark:border-gray-800">
                  <td className={`${stickyCol} px-2 py-1.5 font-mono text-[11px] text-gray-800 dark:text-gray-200 max-w-[160px] truncate`} title={row.account}>
                    {row.account}
                  </td>
                  {row.amounts.map((cell, i) => <BalanceCell key={i} amounts={cell} />)}
                  <BalanceCell amounts={row.total} bold />
                </tr>
              ))}
            </tbody>
            <tfoot>
              <tr className="border-t-2 border-gray-300 dark:border-gray-600">
                <td className={`${stickyCol} px-2 py-1.5 font-semibold text-gray-900 dark:text-gray-100`}>Total</td>
                {report.totals.map((cell, i) => <BalanceCell key={i} amounts={cell} bold />)}
                <BalanceCell amounts={grandTotal} bold />
              </tr>
            </tfoot>
          </table>
        </div>
      ) : null}
    </div>
  );
}

function BudgetView({ dateFrom, dateTo, currency }: { dateFrom: string; dateTo: string; currency: string }) {
  const [budgetRows, setBudgetRows] = useState<BudgetRow[]>([]);
  const [inactive, setInactive] = useState<InactiveBudget[]>([]);
  const [range, setRange] = useState<{ from: string; to: string } | null>(null);
  const [budgetChart, setBudgetChart] = useState<BudgetSummaryPoint[]>([]);
  // Rules are fetched alongside so an empty report can say whether there are
  // simply no rules, or rules the engine had to skip.
  const [rules, setRules] = useState<ForecastRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const loadSeq = useRef(0);

  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    setLoading(true);
    setError(null);
    const params: ReportParams = {
      targetCommodity: currency,
      dateFrom: dateFrom || null,
      dateTo: dateTo || null,
    };
    try {
      const [rows, chart, ruleList] = await Promise.all([
        api.budgetVsActual(params),
        api.budgetSummaryChart(params),
        api.getForecastRules(),
      ]);
      if (seq !== loadSeq.current) return;
      setBudgetRows(rows.rows);
      setInactive(rows.inactive);
      setRange({ from: rows.from, to: rows.to });
      setBudgetChart(chart);
      setRules(ruleList);
    } catch (err) {
      if (seq !== loadSeq.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
  }, [dateFrom, dateTo, currency]);

  useEffect(() => { load(); }, [load]);

  if (loading) {
    return <div className="text-sm text-gray-500 text-center py-8">Loading...</div>;
  }

  if (error) {
    return (
      <div className="space-y-2 py-4">
        <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">{error}</div>
        <button onClick={load} className="w-full py-2 text-sm text-blue-600 font-medium">Retry</button>
      </div>
    );
  }

  const inactiveNotice = inactive.length > 0 && (
    <div className="bg-amber-50 dark:bg-amber-900/20 rounded-lg px-3 py-2 space-y-1">
      <div className="text-xs font-medium text-amber-700 dark:text-amber-400">
        {inactive.length} budget{inactive.length === 1 ? "" : "s"} outside{" "}
        {range ? `${range.from} to ${range.to}` : "this range"}
      </div>
      {inactive.map((b, i) => (
        <div key={i} className="text-xs text-amber-700 dark:text-amber-400 break-words">
          {b.accounts.join(", ") || b.description || `line ${b.line}`}
          {b.starts ? ` starts ${b.starts}` : ` (${b.period})`}
        </div>
      ))}
      <div className="text-xs text-amber-700/80 dark:text-amber-400/80">
        Reports cover the dates your journal spans, so a budget starting later
        has no goal yet. Widen the date filter to include it.
      </div>
    </div>
  );

  if (budgetRows.length === 0) {
    const broken = rules.filter((r) => r.error);
    return (
      <div className="py-8 space-y-2">
        <div className="text-sm text-gray-500 dark:text-gray-400 text-center">
          {rules.length === 0 ? "No budgets defined" : "No budget goals in this date range"}
        </div>
        {inactiveNotice}
        {broken.length > 0 ? (
          <div className="bg-amber-50 dark:bg-amber-900/20 rounded-lg px-3 py-2 space-y-1">
            <div className="text-xs font-medium text-amber-700 dark:text-amber-400">
              {broken.length} rule{broken.length === 1 ? "" : "s"} could not be used:
            </div>
            {broken.map((r, i) => (
              <div key={i} className="text-xs text-amber-700 dark:text-amber-400 break-words">
                line {r.line}: {r.error}
              </div>
            ))}
          </div>
        ) : (
          <p className="text-xs text-gray-400 dark:text-gray-500 text-center">
            {rules.length === 0
              ? "Add periodic transactions (~ monthly) to your journal, or use Settings > Manage Budget"
              : "Your rules parsed fine but produced no goals in this date range — try widening it."}
          </p>
        )}
      </div>
    );
  }

  // Summary totals: group per commodity and exclude income (negative-budget)
  // rows so the "Spent"/"Budgeted" cards only cover expense budgets.
  const expenseRows = budgetRows.filter((r) => parseFloat(r.budget) >= 0);
  const incomeRowCount = budgetRows.length - expenseRows.length;
  const totalsByCommodity = new Map<string, { budget: number; actual: number }>();
  for (const r of expenseRows) {
    const t = totalsByCommodity.get(r.commodity) ?? { budget: 0, actual: 0 };
    t.budget += parseFloat(r.budget);
    t.actual += parseFloat(r.actual);
    totalsByCommodity.set(r.commodity, t);
  }
  const commodityTotals = [...totalsByCommodity.entries()]
    .sort((a, b) => Math.abs(b[1].budget) - Math.abs(a[1].budget));
  const mainTotal = commodityTotals[0];
  const extraCommodities = commodityTotals.length - 1;

  const chartData = budgetChart.map((p) => ({
    period: p.period,
    budgeted: parseFloat(p.budgeted),
    actual: parseFloat(p.actual),
  }));

  return (
    <div className="space-y-4">
      {inactiveNotice}

      {/* Summary cards (expense budgets, dominant commodity) */}
      {mainTotal && (
        <div>
          <div className="grid grid-cols-3 gap-2">
            <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 text-center">
              <div className="text-xs text-gray-500 dark:text-gray-400">Budgeted</div>
              <div className="text-sm font-semibold text-gray-900 dark:text-gray-100 font-mono">
                {fmtBudgetAmt(mainTotal[1].budget.toString(), mainTotal[0])}
              </div>
            </div>
            <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 text-center">
              <div className="text-xs text-gray-500 dark:text-gray-400">Spent</div>
              <div className="text-sm font-semibold text-gray-900 dark:text-gray-100 font-mono">
                {fmtBudgetAmt(mainTotal[1].actual.toString(), mainTotal[0])}
              </div>
            </div>
            <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 text-center">
              <div className="text-xs text-gray-500 dark:text-gray-400">Remaining</div>
              <div className={`text-sm font-semibold font-mono ${mainTotal[1].budget - mainTotal[1].actual >= 0 ? "text-green-500" : "text-red-500"}`}>
                {fmtBudgetAmt((mainTotal[1].budget - mainTotal[1].actual).toString(), mainTotal[0])}
              </div>
            </div>
          </div>
          {(extraCommodities > 0 || incomeRowCount > 0) && (
            <p className="text-xs text-gray-400 dark:text-gray-500 mt-1 text-center">
              {extraCommodities > 0 && `+ ${extraCommodities} more ${extraCommodities === 1 ? "currency" : "currencies"} not shown`}
              {extraCommodities > 0 && incomeRowCount > 0 && " · "}
              {incomeRowCount > 0 && "income budgets excluded from totals"}
            </p>
          )}
        </div>
      )}

      {/* Budget vs Actual table */}
      <div>
        <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">Budget vs Actual</h2>
        <div className="bg-gray-50 dark:bg-gray-800 rounded-lg divide-y divide-gray-200 dark:divide-gray-700">
          {budgetRows.map((row, i) => {
            const pct = parseFloat(row.percentage);
            const barWidth = Math.min(pct, 100);
            return (
              <div key={i} className="px-3 py-2.5">
                <div className="flex justify-between items-center mb-1">
                  <span className="text-sm text-gray-800 dark:text-gray-200 truncate min-w-0" title={row.account}>
                    {row.account.split(":").pop()}
                    {range && (row.periodFrom !== range.from || row.periodTo !== range.to) && (
                      <span className="ml-1 text-[10px] text-gray-400">
                        {row.periodFrom} to {row.periodTo}
                      </span>
                    )}
                  </span>
                  <span className={`shrink-0 ml-2 text-xs font-mono ${row.overBudget ? "text-red-500" : "text-green-500"}`}>
                    {fmtBudgetAmt(row.actual, row.commodity)} / {fmtBudgetAmt(row.budget, row.commodity)}
                  </span>
                </div>
                {/* Progress bar */}
                <div className="h-2 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
                  <div
                    className={`h-full rounded-full transition-all ${row.overBudget ? "bg-red-500" : "bg-green-500"}`}
                    style={{ width: `${barWidth}%` }}
                  />
                </div>
                <div className="flex justify-between mt-0.5">
                  <span className="text-xs text-gray-400">{row.percentage}</span>
                  <span className={`text-xs ${parseFloat(row.difference) >= 0 ? "text-green-500" : "text-red-500"}`}>
                    {parseFloat(row.difference) >= 0 ? "+" : ""}{fmtBudgetAmt(row.difference, row.commodity)} left
                  </span>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* Budget vs Actual chart */}
      {chartData.length > 1 && (
        <div>
          <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">Budget vs Actual Over Time</h2>
          <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-2">
            <ResponsiveContainer width="100%" height={200}>
              <BarChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="#4b5563" />
                <XAxis dataKey="period" tick={{ fontSize: 10, fill: "#9ca3af" }} />
                <YAxis tick={{ fontSize: 10, fill: "#9ca3af" }} width={60} />
                <Tooltip contentStyle={{ backgroundColor: "#1f2937", border: "none", borderRadius: 8, color: "#f3f4f6" }} />
                <Legend wrapperStyle={{ fontSize: 11 }} />
                <Bar dataKey="budgeted" fill="#6366f1" name="Budget" radius={[2,2,0,0]} />
                <Bar dataKey="actual" fill="#f59e0b" name="Actual" radius={[2,2,0,0]} />
              </BarChart>
            </ResponsiveContainer>
          </div>
        </div>
      )}
    </div>
  );
}

/** First and last day of a "YYYY-MM" period label. */
function monthRange(period: string): { from: string; to: string } {
  const [y, m] = period.split("-").map(Number);
  const last = new Date(y, m, 0).getDate();
  return { from: `${period}-01`, to: `${period}-${String(last).padStart(2, "0")}` };
}

const HORIZON_OPTIONS: [number, string][] = [[3, "3m"], [6, "6m"], [12, "1y"], [24, "2y"]];

/** N months from today as YYYY-MM-DD, in local time (toISOString would shift). */
function horizonDate(months: number): string {
  const now = new Date();
  const d = new Date(now.getFullYear(), now.getMonth() + months, now.getDate());
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  return `${d.getFullYear()}-${mm}-${dd}`;
}

function ForecastView({ accountList, dateFrom, dateTo, currency }: { accountList: string[]; dateFrom: string; dateTo: string; currency: string }) {
  // Empty means "let the backend pick every asset account by type".
  const [account, setAccount] = useState("");
  const [months, setMonths] = useState(12);
  const [projection, setProjection] = useState<ForecastProjection | null>(null);
  const [upcoming, setUpcoming] = useState<TransactionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const loadSeq = useRef(0);

  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    setLoading(true);
    setError(null);
    const horizon = horizonDate(months);
    const params: ReportParams = {
      targetCommodity: currency,
      dateFrom: dateFrom || null,
      dateTo: dateTo || null,
    };
    try {
      const [proj, up] = await Promise.all([
        api.forecastProjection(account || null, horizon, params),
        api.upcomingTransactions(horizon, 50),
      ]);
      if (seq !== loadSeq.current) return;
      setProjection(proj);
      setUpcoming(up);
    } catch (err) {
      if (seq !== loadSeq.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
  }, [account, months, dateFrom, dateTo, currency]);

  useEffect(() => { load(); }, [load]);

  const controls = (
    <div className="space-y-2">
      <select value={account} onChange={(e) => setAccount(e.target.value)}
        className="w-full min-w-0 truncate px-3 py-2 min-h-[48px] bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100">
        <option value="">All assets</option>
        {accountList.map((n) => <option key={n} value={n}>{n}</option>)}
      </select>
      <div className="flex items-center gap-2">
        <span className="text-xs text-gray-500 dark:text-gray-400 shrink-0">Look ahead</span>
        <div className="flex gap-1.5 flex-1 min-w-0">
          {HORIZON_OPTIONS.map(([m, label]) => (
            <button key={m} onClick={() => setMonths(m)}
              className={`flex-1 py-2 text-xs font-medium rounded-lg ${m === months ? "bg-blue-600 text-white" : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400"}`}>
              {label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );

  if (error) {
    return (
      <div className="space-y-3">
        {controls}
        <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg break-words">{error}</div>
        <button onClick={load} className="w-full py-2 text-sm text-blue-600 font-medium">Retry</button>
      </div>
    );
  }

  if (loading || !projection) {
    return (
      <div className="space-y-3">
        {controls}
        <div className="text-sm text-gray-500 text-center py-8">Loading...</div>
      </div>
    );
  }

  if (projection.noRules) {
    return (
      <div className="space-y-3">
        {controls}
        <div className="text-center py-8 space-y-2">
          <div className="text-sm text-gray-500 dark:text-gray-400">No recurring transactions yet</div>
          <p className="text-xs text-gray-400 dark:text-gray-500">
            Add rent, salary or subscriptions in Settings &gt; Manage Recurring to project your balance forward
          </p>
        </div>
      </div>
    );
  }

  const { points, shortfall, lastActual, commodity, daysSinceLastActual, ruleErrors } = projection;
  const stale = (daysSinceLastActual ?? 0) > 45;
  const cutoff = lastActual ? lastActual.slice(0, 7) : null;
  // The last actual point is repeated on the projected series so the two
  // lines meet instead of leaving a gap at the cutoff.
  const chartData = points.map((p, i) => {
    const closing = parseFloat(p.closing);
    const bridge = !p.projected && points[i + 1]?.projected === true;
    return {
      period: p.period,
      actual: p.projected ? null : closing,
      projected: p.projected || bridge ? closing : null,
    };
  });
  const hasCutoff = cutoff !== null && points.some((p) => p.period === cutoff);
  const last = points[points.length - 1];

  return (
    <div className="space-y-4 min-w-0">
      {controls}

      {ruleErrors.length > 0 && (
        <div className="rounded-lg px-3 py-2 bg-amber-50 dark:bg-amber-900/20 space-y-1">
          <div className="text-xs font-medium text-amber-700 dark:text-amber-400">
            Some recurring rules generated nothing:
          </div>
          {ruleErrors.map((e, i) => (
            <div key={i} className="text-xs text-amber-700 dark:text-amber-400 break-words">{e}</div>
          ))}
        </div>
      )}

      {stale && (
        <div className="rounded-lg px-3 py-2 bg-amber-50 dark:bg-amber-900/20 text-xs text-amber-700 dark:text-amber-400 break-words">
          Your last recorded transaction was {lastActual}. The projection starts
          from today using that balance, so it assumes nothing has happened since.
        </div>
      )}

      {shortfall && (
        <div className="rounded-lg p-3 bg-red-50 dark:bg-red-900/30 border border-red-300 dark:border-red-700 space-y-1">
          <div className="text-sm font-semibold text-red-700 dark:text-red-400">
            You run out of money on {shortfall.date}
          </div>
          <div className="text-sm text-red-600 dark:text-red-400 font-mono">
            Projected balance {fmtBudgetAmt(shortfall.balance, commodity)}
          </div>
          <div className="text-xs text-red-600/80 dark:text-red-400/80 break-words">
            after &ldquo;{shortfall.description}&rdquo;
          </div>
        </div>
      )}

      {!shortfall && last && (
        <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 text-center">
          <div className="text-xs text-gray-500 dark:text-gray-400">Projected balance by {last.period}</div>
          <div className={`text-lg font-semibold font-mono ${parseFloat(last.closing) < 0 ? "text-red-500" : "text-green-500"}`}>
            {fmtBudgetAmt(last.closing, commodity)}
          </div>
        </div>
      )}

      {chartData.length > 1 && (
        <div>
          <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">Projected Balance</h2>
          <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-2">
            <ResponsiveContainer width="100%" height={200}>
              <LineChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="#4b5563" />
                <XAxis dataKey="period" tick={{ fontSize: 10, fill: "#9ca3af" }} />
                <YAxis tick={{ fontSize: 10, fill: "#9ca3af" }} width={60} />
                <Tooltip contentStyle={{ backgroundColor: "#1f2937", border: "none", borderRadius: 8, color: "#f3f4f6" }} />
                <Legend wrapperStyle={{ fontSize: 11 }} />
                <ReferenceLine y={0} stroke="#6b7280" />
                {hasCutoff && <ReferenceLine x={cutoff ?? undefined} stroke="#f59e0b" strokeDasharray="4 4" />}
                <Line type="monotone" dataKey="actual" name="Actual" stroke="#3b82f6" strokeWidth={2} dot={false} connectNulls={false} />
                <Line type="monotone" dataKey="projected" name="Projected" stroke="#8b5cf6" strokeWidth={2} strokeDasharray="5 4" dot={false} connectNulls={false} />
              </LineChart>
            </ResponsiveContainer>
          </div>
          {hasCutoff && (
            <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">
              Everything after {cutoff} is projected from your recurring transactions
            </p>
          )}
        </div>
      )}

      <div>
        <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">Upcoming</h2>
        {upcoming.length === 0 ? (
          <div className="text-sm text-gray-500 text-center py-4">Nothing projected before the horizon</div>
        ) : (
          <div className="divide-y divide-gray-100 dark:divide-gray-800 min-w-0">
            {upcoming.map((txn, i) => {
              const posting = txn.postings.find((p) => p.amount !== null);
              return (
                // Projections have no journal entry behind them, so these rows
                // are inert — never pass their index to the editor.
                <div key={i} className="py-2.5 flex justify-between items-center gap-2 min-w-0">
                  <div className="min-w-0 flex-1">
                    <div className="text-sm text-gray-900 dark:text-gray-100 truncate flex items-center gap-1.5" title={txn.description}>
                      <span className="shrink-0 text-[10px] font-semibold uppercase tracking-wide px-1.5 py-0.5 rounded bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-400">
                        proj
                      </span>
                      <span className="truncate">{txn.description}</span>
                    </div>
                    <div className="text-xs text-gray-500 dark:text-gray-400">{txn.date}</div>
                  </div>
                  {posting && posting.amount !== null && (
                    <div className={`text-sm font-mono shrink-0 ${parseFloat(posting.amount) < 0 ? "text-red-500" : "text-green-500"}`}>
                      {fmtCompactEntry({ commodity: posting.commodity ?? "", quantity: posting.amount })}
                    </div>
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

export function ReportsPage() {
  const { defaultCurrency } = useSettingsStore();
  const refreshJournal = useJournalStore((s) => s.refresh);
  const navIntent = useNavStore((s) => s.intent);
  const clearNavIntent = useNavStore((s) => s.clearIntent);
  const navigate = useNavStore((s) => s.navigate);
  const goBack = useNavStore((s) => s.goBack);
  const canGoBack = useNavStore((s) => s.history.length > 0);
  const [tab, setTab] = useState<ReportTab>("overview");
  const [registerAccount, setRegisterAccount] = useState("");
  // Period a statement was opened for, when it differs from the page filter.
  const [statementRange, setStatementRange] = useState<{ from: string; to: string } | null>(null);
  const [drillView, setDrillView] = useState<DrillView>(null);
  const [statement, setStatement] = useState<FinancialStatement | null>(null);
  const [netWorth, setNetWorth] = useState<TimeSeriesPoint[]>([]);
  const [incomeExpense, setIncomeExpense] = useState<IncomeExpensePoint[]>([]);
  const [expenseBreakdown, setExpenseBreakdown] = useState<PieSlice[]>([]);
  const [loading, setLoading] = useState(true);
  const [dateFrom, setDateFrom] = useState("");
  const [dateTo, setDateTo] = useState("");
  const [accountList, setAccountList] = useState<string[]>([]);
  const [selectedAccount, setSelectedAccount] = useState("");
  const [accountSeries, setAccountSeries] = useState<TimeSeriesPoint[]>([]);
  const [expensePrefix, setExpensePrefix] = useState<string | null>(null);
  const [expensePath, setExpensePath] = useState<string[]>([]);
  const [pageError, setPageError] = useState<string | null>(null);
  const [drillHint, setDrillHint] = useState<string | null>(null);
  const [valuation, setValuation] = useState<api.ValuationInfo | null>(null);
  const [forecast, setForecast] = useState(false);
  const dashboardSeq = useRef(0);
  const seriesSeq = useRef(0);
  const drillSeq = useRef(0);

  const makeParams = useCallback((): ReportParams => ({
    targetCommodity: defaultCurrency,
    dateFrom: dateFrom || null,
    dateTo: dateTo || null,
  }), [defaultCurrency, dateFrom, dateTo]);

  const loadDashboard = useCallback(async () => {
    const seq = ++dashboardSeq.current;
    setLoading(true);
    setPageError(null);
    const params = makeParams();
    const chartParams: ReportParams = forecast ? { ...params, forecast: true } : params;
    try {
      const [nw, ie, eb, accounts, vi] = await Promise.all([
        api.netWorthSeries(chartParams),
        api.incomeExpenseChart(chartParams),
        api.expenseBreakdownChart(params, null),
        api.listAccountsWithBalances(),
        api.valuationInfo(params),
      ]);
      if (seq !== dashboardSeq.current) return;
      setNetWorth(nw);
      setIncomeExpense(ie);
      setExpenseBreakdown(eb);
      setExpensePrefix(null);
      setExpensePath([]);
      setDrillHint(null);
      setValuation(vi);
      setAccountList(accounts.map((a: BalanceRow) => a.account).sort());
    } catch (err) {
      if (seq !== dashboardSeq.current) return;
      setPageError(err instanceof Error ? err.message : String(err));
    } finally {
      if (seq === dashboardSeq.current) setLoading(false);
    }
  }, [makeParams, forecast]);

  useEffect(() => { loadDashboard(); }, [loadDashboard]);

  useEffect(() => {
    if (selectedAccount) {
      const seq = ++seriesSeq.current;
      api.accountBalanceSeries(selectedAccount, makeParams())
        .then((data) => { if (seq === seriesSeq.current) setAccountSeries(data); })
        .catch((err) => {
          if (seq === seriesSeq.current) setPageError(err instanceof Error ? err.message : String(err));
        });
    }
  }, [selectedAccount, makeParams]);

  const drillIntoExpense = async (category: string) => {
    const seq = ++drillSeq.current;
    const newPrefix = expensePrefix ? `${expensePrefix}:${category}` : `expenses:${category}`;
    try {
      const sub = await api.expenseBreakdownChart(makeParams(), newPrefix);
      if (seq !== drillSeq.current) return;
      // Only drill down if there are real subcategories (not just "other")
      const hasRealSubs = sub.length > 1 || (sub.length === 1 && sub[0].name !== "other");
      if (hasRealSubs) {
        setExpensePrefix(newPrefix);
        setExpensePath((prev) => [...prev, category]);
        setExpenseBreakdown(sub);
        setDrillHint(null);
      } else {
        setDrillHint(category === "other"
          ? "\"other\" aggregates smaller categories — open Accounts for detail"
          : "No further breakdown for this category");
      }
    } catch (err) {
      if (seq !== drillSeq.current) return;
      setPageError(err instanceof Error ? err.message : String(err));
    }
  };

  const expenseBreadcrumbBack = async (index: number) => {
    const seq = ++drillSeq.current;
    const newPath = index < 0 ? [] : expensePath.slice(0, index + 1);
    const newPrefix = newPath.length > 0 ? "expenses:" + newPath.join(":") : null;
    try {
      const eb = await api.expenseBreakdownChart(makeParams(), newPrefix);
      if (seq !== drillSeq.current) return;
      setExpensePrefix(newPrefix);
      setExpensePath(newPath);
      setExpenseBreakdown(eb);
      setDrillHint(null);
    } catch (err) {
      if (seq !== drillSeq.current) return;
      setPageError(err instanceof Error ? err.message : String(err));
    }
  };

  useEffect(() => {
    if (!navIntent) return;
    if (navIntent.kind === "register") {
      setRegisterAccount(navIntent.account);
      if (navIntent.dateFrom !== undefined) setDateFrom(navIntent.dateFrom);
      if (navIntent.dateTo !== undefined) setDateTo(navIntent.dateTo);
      setDrillView(null);
      setTab("register");
    } else if (navIntent.kind === "report-tab") {
      setDrillView(null);
      setTab(navIntent.tab);
    } else if (navIntent.kind === "income-statement") {
      const range =
        navIntent.dateFrom && navIntent.dateTo
          ? { from: navIntent.dateFrom, to: navIntent.dateTo }
          : undefined;
      setTab("overview");
      openStatement("income-statement", range);
    }
    clearNavIntent();
    // openStatement is recreated each render; re-running on it would loop.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [navIntent, clearNavIntent]);

  const setRegisterFor = (account: string) => {
    navigate("reports", { kind: "register", account }, { tab: "reports", reportTab: tab });
  };

  const openStatement = async (type: DrillView, range?: { from: string; to: string }) => {
    if (!type) return;
    const params = range
      ? { ...makeParams(), dateFrom: range.from, dateTo: range.to }
      : makeParams();
    try {
      const data = type === "balance-sheet" ? await api.balanceSheetReport(params)
        : type === "income-statement" ? await api.incomeStatementReport(params)
        : await api.cashFlowReport(params);
      setStatement(data);
      setStatementRange(range ?? null);
      setDrillView(type);
    } catch (err) {
      setPageError(err instanceof Error ? err.message : String(err));
    }
  };

  if (drillView && statement) {
    return (
      <StatementView
        statement={statement}
        subtitle={statementRange ? `${statementRange.from} to ${statementRange.to}` : null}
        onBack={() => { setDrillView(null); setStatementRange(null); }}
      />
    );
  }

  const nwData = netWorth.map((p) => ({ date: p.date.slice(0, 7), value: parseFloat(p.value) }));
  const ieData = incomeExpense.map((p) => ({ period: p.period, income: parseFloat(p.income), expenses: parseFloat(p.expenses) }));
  const pieData = expenseBreakdown.map((s) => ({ name: s.name, value: parseFloat(s.value) }));
  const pieTotal = pieData.reduce((sum, d) => sum + d.value, 0);
  const acctData = accountSeries.map((p) => ({ date: p.date.slice(0, 7), value: parseFloat(p.value) }));

  return (
    <div className="flex flex-col h-full">
      <div className="px-4 py-3 border-b border-gray-200 dark:border-gray-700 space-y-2">
        <div className="flex items-center gap-2 min-w-0">
          {canGoBack && (
            <button onClick={goBack} className="p-1 -ml-1 text-blue-600 dark:text-blue-400 text-sm shrink-0">
              &larr; Back
            </button>
          )}
          <h1 className="text-lg font-semibold text-gray-900 dark:text-gray-100 truncate">Reports</h1>
        </div>
        <div className="flex gap-1">
          {([["overview", "Overview"], ["table", "Table"], ["register", "Register"], ["budget", "Budget"], ["forecast", "Forecast"]] as [ReportTab, string][]).map(([t, label]) => (
            <button key={t} onClick={() => setTab(t)}
              className={`flex-1 min-w-0 truncate py-2 text-sm font-medium rounded-lg ${t === tab ? "bg-blue-600 text-white" : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400"}`}>
              {label}
            </button>
          ))}
        </div>
        <DateFilter dateFrom={dateFrom} dateTo={dateTo} onChange={(f, t) => { setDateFrom(f); setDateTo(t); }} />
        {tab === "overview" && (
          <label className="flex items-center gap-1.5 text-xs text-gray-600 dark:text-gray-400">
            <input type="checkbox" checked={forecast} onChange={(e) => setForecast(e.target.checked)} className="accent-blue-600" />
            Forecast
          </label>
        )}
      </div>

      <div className="flex-1 overflow-y-auto overflow-x-hidden">
        {pageError && (
          <div className="mx-4 mt-3 flex items-center justify-between text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 px-3 py-2 rounded-lg">
            <span className="min-w-0 break-words">{pageError}</span>
            <button onClick={() => setPageError(null)} className="text-xs text-red-500 ml-2 shrink-0">Dismiss</button>
          </div>
        )}
        {valuation && valuation.unconvertible.length > 0 && tab === "overview" && (
          <div className="mx-4 mt-3 text-xs text-amber-700 dark:text-amber-400 bg-amber-50 dark:bg-amber-900/20 px-3 py-2 rounded-lg">
            Charts are valued in {valuation.targetCommodity || "the journal's main currency"}. No price is known for{" "}
            {valuation.unconvertible.join(", ")} — holdings in{" "}
            {valuation.unconvertible.length === 1 ? "this commodity are" : "these commodities are"} not included in
            chart totals. Add P price directives to include them.
          </div>
        )}
        {tab === "budget" ? (
          <div className="p-4"><BudgetView dateFrom={dateFrom} dateTo={dateTo} currency={defaultCurrency} /></div>
        ) : tab === "forecast" ? (
          <div className="p-4"><ForecastView accountList={accountList} dateFrom={dateFrom} dateTo={dateTo} currency={defaultCurrency} /></div>
        ) : tab === "table" ? (
          <div className="p-4"><TableView dateFrom={dateFrom} dateTo={dateTo} /></div>
        ) : tab === "register" ? (
          <div className="p-4"><RegisterView accountList={accountList} account={registerAccount} onAccountChange={setRegisterAccount} dateFrom={dateFrom} dateTo={dateTo} currency={defaultCurrency} onChanged={() => { refreshJournal(); loadDashboard(); }} /></div>
        ) : loading ? (
          <div className="flex items-center justify-center h-32 text-gray-500 text-sm">Loading...</div>
        ) : (
          <div className="p-4 space-y-6">
            {/* Statement links */}
            <div className="grid grid-cols-3 gap-2">
              {([["balance-sheet","Balance Sheet"],["income-statement","Income Stmt"],["cash-flow","Cash Flow"]] as [DrillView,string][]).map(([type,label]) => (
                <button key={type} onClick={() => openStatement(type)}
                  className="py-3 bg-gray-50 dark:bg-gray-800 rounded-lg text-sm font-medium text-gray-700 dark:text-gray-300 active:bg-gray-100 dark:active:bg-gray-700">{label}</button>
              ))}
            </div>

            {/* Net Worth */}
            {nwData.length > 0 && (
              <div>
                <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">Net Worth</h2>
                <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-2">
                  <ResponsiveContainer width="100%" height={180}>
                    <LineChart data={nwData}>
                      <CartesianGrid strokeDasharray="3 3" stroke="#4b5563" />
                      <XAxis dataKey="date" tick={{ fontSize: 10, fill: "#9ca3af" }} />
                      <YAxis tick={{ fontSize: 10, fill: "#9ca3af" }} width={60} />
                      <Tooltip contentStyle={{ backgroundColor: "#1f2937", border: "none", borderRadius: 8, color: "#f3f4f6" }} />
                      <Line type="monotone" dataKey="value" stroke="#3b82f6" strokeWidth={2} dot={false} />
                    </LineChart>
                  </ResponsiveContainer>
                </div>
                {forecast && <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">includes forecast from periodic transactions</p>}
              </div>
            )}

            {/* Income vs Expenses */}
            {ieData.length > 0 && (
              <div>
                <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">Income vs Expenses</h2>
                <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-2">
                  <ResponsiveContainer width="100%" height={200}>
                    <BarChart
                      data={ieData}
                      stackOffset="sign"
                      style={{ cursor: "pointer" }}
                      onClick={(e) => {
                        const period = e?.activeLabel;
                        if (typeof period === "string" && period)
                          openStatement("income-statement", monthRange(period));
                      }}
                    >
                      <CartesianGrid strokeDasharray="3 3" stroke="#4b5563" />
                      <XAxis dataKey="period" tick={{ fontSize: 10, fill: "#9ca3af" }} />
                      <YAxis tick={{ fontSize: 10, fill: "#9ca3af" }} width={60} />
                      <Tooltip contentStyle={{ backgroundColor: "#1f2937", border: "none", borderRadius: 8, color: "#f3f4f6" }} />
                      <Legend wrapperStyle={{ fontSize: 11 }} />
                      <ReferenceLine y={0} stroke="#6b7280" />
                      <Bar dataKey="income" fill="#22c55e" name="Income" stackId="s" radius={[2,2,0,0]} />
                      <Bar dataKey="expenses" fill="#ef4444" name="Expenses" stackId="s" radius={[0,0,2,2]} />
                    </BarChart>
                  </ResponsiveContainer>
                </div>
                <p className="text-xs text-gray-400 dark:text-gray-500 mt-1 text-center">Tap a month for its income statement</p>
                {forecast && <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">includes forecast from periodic transactions</p>}
              </div>
            )}

            {/* Expense Breakdown */}
            {pieData.length > 0 && (
              <div>
                <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">Expense Breakdown</h2>
                {expensePath.length > 0 && (
                  <div className="flex items-center gap-1 mb-2 text-xs flex-wrap">
                    <button onClick={() => expenseBreadcrumbBack(-1)} className="text-blue-500">All</button>
                    {expensePath.map((part, i) => (
                      <span key={i} className="flex items-center gap-1">
                        <span className="text-gray-400">/</span>
                        <button onClick={() => expenseBreadcrumbBack(i)}
                          className={i === expensePath.length - 1 ? "text-gray-700 dark:text-gray-300 font-medium" : "text-blue-500"}>{part}</button>
                      </span>
                    ))}
                  </div>
                )}
                <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3">
                  <ResponsiveContainer width="100%" height={160}>
                    <PieChart>
                      <Pie data={pieData} cx="50%" cy="50%" innerRadius={35} outerRadius={65} dataKey="value"
                        onClick={(_, index) => drillIntoExpense(pieData[index].name)} style={{ cursor: "pointer" }}>
                        {pieData.map((_, i) => <Cell key={i} fill={COLORS[i % COLORS.length]} />)}
                      </Pie>
                      <Tooltip contentStyle={{ backgroundColor: "#1f2937", border: "none", borderRadius: 8, color: "#f3f4f6" }}
                        formatter={(value) => Number(value).toLocaleString(undefined, { minimumFractionDigits: 2 })} />
                    </PieChart>
                  </ResponsiveContainer>
                  <div className="mt-2 divide-y divide-gray-200 dark:divide-gray-700 min-w-0">
                    {pieData.map((item, i) => {
                      const pct = pieTotal > 0 ? ((item.value / pieTotal) * 100).toFixed(0) : "0";
                      // "other" aggregates the small categories, so there is no
                      // single account whose register we could open.
                      const isAggregate = item.name === "other";
                      const account = expensePrefix ? `${expensePrefix}:${item.name}` : `expenses:${item.name}`;
                      return (
                        <button
                          key={item.name}
                          disabled={isAggregate}
                          onClick={() => setRegisterFor(account)}
                          title={isAggregate ? "Aggregate of smaller categories" : `Show ${account} transactions`}
                          className="w-full py-2 flex items-center gap-2 min-w-0 text-left disabled:opacity-60"
                        >
                          <span className="w-2.5 h-2.5 rounded-sm shrink-0" style={{ backgroundColor: COLORS[i % COLORS.length] }} />
                          <span className="flex-1 min-w-0 truncate text-sm text-gray-800 dark:text-gray-200">{item.name}</span>
                          <span className="text-xs text-gray-400 shrink-0 w-9 text-right">{pct}%</span>
                          <span className="text-sm font-mono shrink-0 text-gray-800 dark:text-gray-200">
                            {fmtBudgetAmt(String(item.value), defaultCurrency)}
                          </span>
                        </button>
                      );
                    })}
                    <div className="py-2 flex items-center gap-2 min-w-0 font-semibold">
                      <span className="w-2.5 shrink-0" />
                      <span className="flex-1 min-w-0 text-sm text-gray-900 dark:text-gray-100">Total</span>
                      <span className="text-sm font-mono shrink-0 text-gray-900 dark:text-gray-100">
                        {fmtBudgetAmt(String(pieTotal), defaultCurrency)}
                      </span>
                    </div>
                  </div>
                  {drillHint && <p className="text-xs text-amber-600 dark:text-amber-400 mt-1 text-center">{drillHint}</p>}
                  <p className="text-xs text-gray-400 mt-1 text-center">Tap a slice to drill down, or a row for its transactions</p>
                </div>
              </div>
            )}

            {/* Account Growth */}
            {accountList.length > 0 && (
              <div>
                <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">Account Balance Over Time</h2>
                <select value={selectedAccount} onChange={(e) => setSelectedAccount(e.target.value)}
                  className="w-full mb-2 px-3 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100">
                  <option value="">Select an account...</option>
                  {accountList.map((n) => <option key={n} value={n}>{n}</option>)}
                </select>
                {selectedAccount && acctData.length > 0 && (
                  <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-2">
                    <ResponsiveContainer width="100%" height={180}>
                      <LineChart data={acctData}>
                        <CartesianGrid strokeDasharray="3 3" stroke="#4b5563" />
                        <XAxis dataKey="date" tick={{ fontSize: 10, fill: "#9ca3af" }} />
                        <YAxis tick={{ fontSize: 10, fill: "#9ca3af" }} width={60} />
                        <Tooltip contentStyle={{ backgroundColor: "#1f2937", border: "none", borderRadius: 8, color: "#f3f4f6" }} />
                        <Line type="monotone" dataKey="value" stroke="#8b5cf6" strokeWidth={2} dot={false} />
                      </LineChart>
                    </ResponsiveContainer>
                  </div>
                )}
              </div>
            )}

            {nwData.length === 0 && ieData.length === 0 && pieData.length === 0 && (
              <div className="text-center text-gray-500 text-sm py-8">Add transactions to see reports</div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
