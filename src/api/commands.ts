import { invoke } from "@tauri-apps/api/core";
import type {
  JournalSummary,
  TransactionSummary,
  NewTransaction,
  ReportParams,
  BalanceRow,
  RegisterRow,
  FinancialStatement,
  TimeSeriesPoint,
  IncomeExpensePoint,
  PieSlice,
  ReconciliationState,
  BudgetRow,
  BudgetSummaryPoint,
  BudgetInfo,
  BudgetEntry,
} from "./types";

// ─── Journal ───

export async function openJournal(path: string): Promise<JournalSummary> {
  return invoke<JournalSummary>("open_journal", { path });
}

export async function getJournalInfo(): Promise<JournalSummary> {
  return invoke<JournalSummary>("get_journal_info");
}

export async function saveJournal(): Promise<void> {
  return invoke<void>("save_journal");
}

export async function createJournal(
  path: string,
  defaultCurrency?: string
): Promise<JournalSummary> {
  return invoke<JournalSummary>("create_journal", {
    path,
    defaultCurrency: defaultCurrency ?? null,
  });
}

export async function addTransaction(
  txn: NewTransaction
): Promise<JournalSummary> {
  return invoke<JournalSummary>("add_transaction", { txn });
}

export async function updateTransaction(
  index: number,
  txn: NewTransaction
): Promise<JournalSummary> {
  return invoke<JournalSummary>("update_transaction", { index, txn });
}

export async function deleteTransaction(
  index: number
): Promise<JournalSummary> {
  return invoke<JournalSummary>("delete_transaction", { index });
}

// ─── Transactions ───

export async function listTransactions(): Promise<TransactionSummary[]> {
  return invoke<TransactionSummary[]>("list_transactions");
}

export async function getTransaction(
  index: number
): Promise<TransactionSummary> {
  return invoke<TransactionSummary>("get_transaction", { index });
}

// ─── Autocomplete ───

export async function suggestAccounts(prefix: string): Promise<string[]> {
  return invoke<string[]>("suggest_accounts", { prefix });
}

export async function suggestDescriptions(prefix: string): Promise<string[]> {
  return invoke<string[]>("suggest_descriptions", { prefix });
}

export async function suggestPayees(prefix: string): Promise<string[]> {
  return invoke<string[]>("suggest_payees", { prefix });
}

// ─── Reports ───

export async function balanceReport(
  params: ReportParams = {}
): Promise<BalanceRow[]> {
  return invoke<BalanceRow[]>("balance_report", { params });
}

export async function registerReport(
  account: string,
  params: ReportParams = {}
): Promise<RegisterRow[]> {
  return invoke<RegisterRow[]>("register_report", { account, params });
}

export async function balanceSheetReport(
  params: ReportParams = {}
): Promise<FinancialStatement> {
  return invoke<FinancialStatement>("balance_sheet_report", { params });
}

export async function incomeStatementReport(
  params: ReportParams = {}
): Promise<FinancialStatement> {
  return invoke<FinancialStatement>("income_statement_report", { params });
}

export async function cashFlowReport(
  params: ReportParams = {}
): Promise<FinancialStatement> {
  return invoke<FinancialStatement>("cash_flow_report", { params });
}

export async function netWorthSeries(
  params: ReportParams = {}
): Promise<TimeSeriesPoint[]> {
  return invoke<TimeSeriesPoint[]>("net_worth_series", { params });
}

export async function accountBalanceSeries(
  account: string,
  params: ReportParams = {}
): Promise<TimeSeriesPoint[]> {
  return invoke<TimeSeriesPoint[]>("account_balance_series", {
    account,
    params,
  });
}

export async function incomeExpenseChart(
  params: ReportParams = {}
): Promise<IncomeExpensePoint[]> {
  return invoke<IncomeExpensePoint[]>("income_expense_chart", { params });
}

export async function expenseBreakdownChart(
  params: ReportParams = {},
  parentPrefix?: string | null
): Promise<PieSlice[]> {
  return invoke<PieSlice[]>("expense_breakdown_chart", {
    params,
    parentPrefix: parentPrefix ?? null,
  });
}

export async function listAccountsWithBalances(): Promise<BalanceRow[]> {
  return invoke<BalanceRow[]>("list_accounts_with_balances");
}

// ─── Budget ───

export async function getBudgets(): Promise<BudgetInfo[]> {
  return invoke<BudgetInfo[]>("get_budgets");
}

export async function budgetVsActual(
  params: ReportParams = {}
): Promise<BudgetRow[]> {
  return invoke<BudgetRow[]>("budget_vs_actual", { params });
}

export async function budgetSummaryChart(
  params: ReportParams = {}
): Promise<BudgetSummaryPoint[]> {
  return invoke<BudgetSummaryPoint[]>("budget_summary_chart", { params });
}

export async function saveBudget(
  entries: BudgetEntry[],
  period: string
): Promise<JournalSummary> {
  return invoke<JournalSummary>("save_budget", { entries, period });
}

export async function listBudgetAccounts(): Promise<string[]> {
  return invoke<string[]>("list_budget_accounts");
}

export async function switchJournal(path: string): Promise<JournalSummary> {
  return invoke<JournalSummary>("switch_journal", { path });
}

// ─── Reconciliation ───

export async function startReconciliation(params: {
  account: string;
  statementDate: string;
  statementBalance: string;
  commodity: string;
}): Promise<ReconciliationState> {
  return invoke<ReconciliationState>("start_reconciliation", { params });
}

export async function toggleReconciliationPosting(
  index: number
): Promise<ReconciliationState> {
  return invoke<ReconciliationState>("toggle_reconciliation_posting", {
    index,
  });
}

export async function finishReconciliation(): Promise<JournalSummary> {
  return invoke<JournalSummary>("finish_reconciliation");
}

export async function cancelReconciliation(): Promise<void> {
  return invoke<void>("cancel_reconciliation");
}
