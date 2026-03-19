import { useState, useEffect, useCallback } from "react";
import {
  LineChart, Line, BarChart, Bar, PieChart, Pie, Cell,
  XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer,
  ReferenceLine, Legend,
} from "recharts";
import * as api from "../api/commands";
import { useSettingsStore } from "../store/settingsStore";
import { DateFilter } from "../components/common/DateFilter";
import type {
  TimeSeriesPoint, IncomeExpensePoint, PieSlice,
  FinancialStatement, BalanceRow, RegisterRow, ReportParams,
  BudgetRow, BudgetSummaryPoint,
} from "../api/types";

type ReportTab = "overview" | "register" | "budget";
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

function StatementView({ statement, onBack }: { statement: FinancialStatement; onBack: () => void }) {
  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        <button onClick={onBack} className="p-2 -ml-2 text-gray-600 dark:text-gray-300">&larr;</button>
        <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">{statement.title}</h2>
      </div>
      <div className="flex-1 overflow-auto p-4 space-y-4">
        {statement.sections.map((section, si) => (
          <div key={si}>
            <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 uppercase tracking-wide mb-2">{section.title}</h3>
            {section.rows.length > 0 ? (
              <div className="bg-gray-50 dark:bg-gray-800 rounded-lg divide-y divide-gray-200 dark:divide-gray-700">
                {section.rows.map((row, ri) => (
                  <div key={ri} className="px-3 py-2 flex justify-between" style={{ paddingLeft: `${12 + row.depth * 16}px` }}>
                    <span className="text-sm text-gray-800 dark:text-gray-200 truncate">{row.account.split(":").pop()}</span>
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

function RegisterView({ accountList, dateFrom, dateTo, currency }: { accountList: string[]; dateFrom: string; dateTo: string; currency: string }) {
  const [account, setAccount] = useState("");
  const [rows, setRows] = useState<RegisterRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [newestFirst, setNewestFirst] = useState(true);

  const load = useCallback(async () => {
    if (!account) { setRows([]); return; }
    setLoading(true);
    const data = await api.registerReport(account, { dateFrom: dateFrom || null, dateTo: dateTo || null, targetCommodity: currency });
    setRows(data);
    setLoading(false);
  }, [account, dateFrom, dateTo, currency]);

  useEffect(() => { load(); }, [load]);

  const displayRows = newestFirst ? [...rows].reverse() : rows;

  return (
    <div className="space-y-3">
      <div className="flex gap-2">
        <select value={account} onChange={(e) => setAccount(e.target.value)}
          className="flex-1 px-3 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg text-sm text-gray-900 dark:text-gray-100">
          <option value="">Select an account...</option>
          {accountList.map((n) => <option key={n} value={n}>{n}</option>)}
        </select>
        <button
          onClick={() => setNewestFirst(!newestFirst)}
          className="px-3 py-2 bg-gray-100 dark:bg-gray-800 rounded-lg text-xs font-medium text-gray-600 dark:text-gray-400 shrink-0"
        >
          {newestFirst ? "New \u2193" : "Old \u2191"}
        </button>
      </div>
      {loading && <div className="text-sm text-gray-500 text-center py-4">Loading...</div>}
      {!loading && account && rows.length === 0 && <div className="text-sm text-gray-500 text-center py-4">No postings</div>}
      {displayRows.length > 0 && (
        <div className="divide-y divide-gray-100 dark:divide-gray-800">
          {displayRows.map((row, i) => (
            <div key={i} className="py-2.5 flex justify-between items-center">
              <div className="min-w-0 flex-1">
                <div className="text-sm text-gray-900 dark:text-gray-100 truncate">{row.description}</div>
                <div className="text-xs text-gray-500 dark:text-gray-400">{row.date}</div>
              </div>
              <div className="text-right ml-3 shrink-0">
                <div className={`text-sm font-mono ${parseFloat(row.amount[0]?.quantity ?? "0") < 0 ? "text-red-500" : "text-green-500"}`}>{fmtAmt(row.amount)}</div>
                <div className="text-xs text-gray-400 font-mono">{fmtAmt(row.runningTotal)}</div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function BudgetView({ dateFrom, dateTo, currency }: { dateFrom: string; dateTo: string; currency: string }) {
  const [budgetRows, setBudgetRows] = useState<BudgetRow[]>([]);
  const [budgetChart, setBudgetChart] = useState<BudgetSummaryPoint[]>([]);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    const params: ReportParams = {
      targetCommodity: currency,
      dateFrom: dateFrom || null,
      dateTo: dateTo || null,
    };
    const [rows, chart] = await Promise.all([
      api.budgetVsActual(params),
      api.budgetSummaryChart(params),
    ]);
    setBudgetRows(rows);
    setBudgetChart(chart);
    setLoading(false);
  }, [dateFrom, dateTo, currency]);

  useEffect(() => { load(); }, [load]);

  if (loading) {
    return <div className="text-sm text-gray-500 text-center py-8">Loading...</div>;
  }

  if (budgetRows.length === 0) {
    return (
      <div className="text-center py-8 space-y-2">
        <div className="text-sm text-gray-500 dark:text-gray-400">No budgets defined</div>
        <p className="text-xs text-gray-400 dark:text-gray-500">
          Add periodic transactions (~ monthly) to your journal or use Settings &gt; Manage Budget
        </p>
      </div>
    );
  }

  // Summary totals
  const totalBudget = budgetRows.reduce((s, r) => s + parseFloat(r.budget), 0);
  const totalActual = budgetRows.reduce((s, r) => s + parseFloat(r.actual), 0);
  const totalRemaining = totalBudget - totalActual;
  const mainCommodity = budgetRows[0]?.commodity ?? currency;

  const chartData = budgetChart.map((p) => ({
    period: p.period,
    budgeted: parseFloat(p.budgeted),
    actual: parseFloat(p.actual),
  }));

  return (
    <div className="space-y-4">
      {/* Summary cards */}
      <div className="grid grid-cols-3 gap-2">
        <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 text-center">
          <div className="text-xs text-gray-500 dark:text-gray-400">Budgeted</div>
          <div className="text-sm font-semibold text-gray-900 dark:text-gray-100 font-mono">
            {fmtBudgetAmt(totalBudget.toString(), mainCommodity)}
          </div>
        </div>
        <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 text-center">
          <div className="text-xs text-gray-500 dark:text-gray-400">Spent</div>
          <div className="text-sm font-semibold text-gray-900 dark:text-gray-100 font-mono">
            {fmtBudgetAmt(totalActual.toString(), mainCommodity)}
          </div>
        </div>
        <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 text-center">
          <div className="text-xs text-gray-500 dark:text-gray-400">Remaining</div>
          <div className={`text-sm font-semibold font-mono ${totalRemaining >= 0 ? "text-green-500" : "text-red-500"}`}>
            {fmtBudgetAmt(totalRemaining.toString(), mainCommodity)}
          </div>
        </div>
      </div>

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
                  <span className="text-sm text-gray-800 dark:text-gray-200 truncate">
                    {row.account.split(":").pop()}
                  </span>
                  <span className={`text-xs font-mono ${row.overBudget ? "text-red-500" : "text-green-500"}`}>
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

export function ReportsPage() {
  const { defaultCurrency } = useSettingsStore();
  const [tab, setTab] = useState<ReportTab>("overview");
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

  const makeParams = useCallback((): ReportParams => ({
    targetCommodity: defaultCurrency,
    dateFrom: dateFrom || null,
    dateTo: dateTo || null,
  }), [defaultCurrency, dateFrom, dateTo]);

  const loadDashboard = useCallback(async () => {
    setLoading(true);
    const params = makeParams();
    const [nw, ie, eb, accounts] = await Promise.all([
      api.netWorthSeries(params),
      api.incomeExpenseChart(params),
      api.expenseBreakdownChart(params, null),
      api.listAccountsWithBalances(),
    ]);
    setNetWorth(nw);
    setIncomeExpense(ie);
    setExpenseBreakdown(eb);
    setExpensePrefix(null);
    setExpensePath([]);
    setAccountList(accounts.map((a: BalanceRow) => a.account).sort());
    setLoading(false);
  }, [makeParams]);

  useEffect(() => { loadDashboard(); }, [loadDashboard]);

  useEffect(() => {
    if (selectedAccount) {
      api.accountBalanceSeries(selectedAccount, makeParams()).then(setAccountSeries);
    }
  }, [selectedAccount, makeParams]);

  const drillIntoExpense = async (category: string) => {
    const newPrefix = expensePrefix ? `${expensePrefix}:${category}` : `expenses:${category}`;
    const sub = await api.expenseBreakdownChart(makeParams(), newPrefix);
    if (sub.length > 0) {
      setExpensePrefix(newPrefix);
      setExpensePath((prev) => [...prev, category]);
      setExpenseBreakdown(sub);
    }
  };

  const expenseBreadcrumbBack = async (index: number) => {
    const newPath = index < 0 ? [] : expensePath.slice(0, index + 1);
    const newPrefix = newPath.length > 0 ? "expenses:" + newPath.join(":") : null;
    setExpensePrefix(newPrefix);
    setExpensePath(newPath);
    const eb = await api.expenseBreakdownChart(makeParams(), newPrefix);
    setExpenseBreakdown(eb);
  };

  const openStatement = async (type: DrillView) => {
    if (!type) return;
    const params = makeParams();
    const data = type === "balance-sheet" ? await api.balanceSheetReport(params)
      : type === "income-statement" ? await api.incomeStatementReport(params)
      : await api.cashFlowReport(params);
    setStatement(data);
    setDrillView(type);
  };

  if (drillView && statement) {
    return <StatementView statement={statement} onBack={() => setDrillView(null)} />;
  }

  const nwData = netWorth.map((p) => ({ date: p.date.slice(0, 7), value: parseFloat(p.value) }));
  const ieData = incomeExpense.map((p) => ({ period: p.period, income: parseFloat(p.income), expenses: parseFloat(p.expenses) }));
  const pieData = expenseBreakdown.map((s) => ({ name: s.name, value: parseFloat(s.value) }));
  const acctData = accountSeries.map((p) => ({ date: p.date.slice(0, 7), value: parseFloat(p.value) }));

  return (
    <div className="flex flex-col h-full">
      <div className="px-4 py-3 border-b border-gray-200 dark:border-gray-700 space-y-2">
        <h1 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Reports</h1>
        <div className="flex gap-1">
          {([["overview", "Overview"], ["register", "Register"], ["budget", "Budget"]] as [ReportTab, string][]).map(([t, label]) => (
            <button key={t} onClick={() => setTab(t)}
              className={`flex-1 py-2 text-sm font-medium rounded-lg ${t === tab ? "bg-blue-600 text-white" : "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400"}`}>
              {label}
            </button>
          ))}
        </div>
        <DateFilter dateFrom={dateFrom} dateTo={dateTo} onChange={(f, t) => { setDateFrom(f); setDateTo(t); }} />
      </div>

      <div className="flex-1 overflow-auto">
        {tab === "budget" ? (
          <div className="p-4"><BudgetView dateFrom={dateFrom} dateTo={dateTo} currency={defaultCurrency} /></div>
        ) : tab === "register" ? (
          <div className="p-4"><RegisterView accountList={accountList} dateFrom={dateFrom} dateTo={dateTo} currency={defaultCurrency} /></div>
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
              </div>
            )}

            {/* Income vs Expenses */}
            {ieData.length > 0 && (
              <div>
                <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">Income vs Expenses</h2>
                <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-2">
                  <ResponsiveContainer width="100%" height={200}>
                    <BarChart data={ieData} stackOffset="sign">
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
                  <div className="flex flex-wrap gap-x-3 gap-y-1 mt-2 justify-center">
                    {pieData.map((item, i) => {
                      const total = pieData.reduce((s, d) => s + d.value, 0);
                      const pct = total > 0 ? ((item.value / total) * 100).toFixed(0) : "0";
                      return (
                        <button key={item.name} onClick={() => drillIntoExpense(item.name)}
                          className="flex items-center gap-1 text-xs text-gray-700 dark:text-gray-300">
                          <span className="w-2.5 h-2.5 rounded-sm shrink-0" style={{ backgroundColor: COLORS[i % COLORS.length] }} />
                          <span className="truncate max-w-[80px]">{item.name}</span>
                          <span className="text-gray-400">{pct}%</span>
                        </button>
                      );
                    })}
                  </div>
                  {expensePath.length === 0 && <p className="text-xs text-gray-400 mt-1 text-center">Tap a slice to drill down</p>}
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
