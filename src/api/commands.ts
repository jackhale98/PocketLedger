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
  BudgetComparison,
  BudgetSummaryPoint,
  CsvPreview,
  CsvImportResult,
  BalanceInterval,
  BalanceAccumulationMode,
  PeriodicBalanceReport,
  PlatformInfo,
  StoredJournal,
  ImportedJournal,
  CreatedJournal,
  ForecastRule,
  SaveForecastPosting,
  ForecastProjection,
  JournalFileInfo,
} from "./types";

// ─── Journal ───

export async function openJournal(path: string): Promise<JournalSummary> {
  return invoke<JournalSummary>("open_journal", { path });
}

export async function getJournalInfo(): Promise<JournalSummary> {
  return invoke<JournalSummary>("get_journal_info");
}

/** True when a source file changed on disk since load (e.g. a git client
 *  pulled into the journal folder). */
export async function journalChangedOnDisk(): Promise<boolean> {
  return invoke<boolean>("journal_changed_on_disk");
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

/** Files making up the loaded journal, for choosing where entries go. */
export async function listJournalFiles(): Promise<JournalFileInfo[]> {
  return invoke<JournalFileInfo[]>("list_journal_files");
}

export async function addTransaction(
  txn: NewTransaction,
  fileIndex?: number
): Promise<JournalSummary> {
  return invoke<JournalSummary>("add_transaction", {
    txn,
    fileIndex: fileIndex ?? null,
  });
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

/** Search transactions with the hledger query language (acct:, desc:, amt:,
 *  date:, cur:, status:, tag:, not:, plus bare terms). Rejects with a
 *  user-readable message on invalid query syntax. */
export async function searchTransactions(
  query: string
): Promise<TransactionSummary[]> {
  return invoke<TransactionSummary[]>("search_transactions", { query });
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

export async function listAccountsWithBalances(
  params?: ReportParams
): Promise<BalanceRow[]> {
  return invoke<BalanceRow[]>("list_accounts_with_balances", {
    params: params ?? null,
  });
}

export async function listCommodities(): Promise<string[]> {
  return invoke<string[]>("list_commodities");
}

export async function periodicBalance(
  interval: BalanceInterval,
  mode: BalanceAccumulationMode | null,
  depth: number | null,
  params: ReportParams = {}
): Promise<PeriodicBalanceReport> {
  return invoke<PeriodicBalanceReport>("periodic_balance", {
    interval,
    mode,
    depth,
    params,
  });
}

// ─── Budget ───

export async function budgetVsActual(
  params: ReportParams = {}
): Promise<BudgetComparison> {
  return invoke<BudgetComparison>("budget_vs_actual", { params });
}

export async function budgetSummaryChart(
  params: ReportParams = {}
): Promise<BudgetSummaryPoint[]> {
  return invoke<BudgetSummaryPoint[]>("budget_summary_chart", { params });
}

export async function switchJournal(path: string): Promise<JournalSummary> {
  return invoke<JournalSummary>("switch_journal", { path });
}

// ─── Storage (mobile journal management) ───

export async function platformInfo(): Promise<PlatformInfo> {
  return invoke<PlatformInfo>("platform_info");
}

/** Resolve a persisted journal reference (relative on mobile) to an absolute
 *  path against the CURRENT storage directory. */
export async function resolveJournalRef(reference: string): Promise<string> {
  return invoke<string>("resolve_journal_ref", { reference });
}

export async function listStoredJournals(): Promise<StoredJournal[]> {
  return invoke<StoredJournal[]>("list_stored_journals");
}

/** Copy a picked file into app storage (out of the iOS picker Inbox, which
 *  the OS deletes between launches). Never overwrites an existing journal. */
export async function importJournalFile(
  path: string
): Promise<ImportedJournal> {
  return invoke<ImportedJournal>("import_journal_file", { path });
}

/** Remove a journal (and its .bak) from app storage. */
export async function deleteStoredJournal(name: string): Promise<void> {
  return invoke<void>("delete_stored_journal", { name });
}

export async function createStoredJournal(
  name: string,
  defaultCurrency?: string
): Promise<CreatedJournal> {
  return invoke<CreatedJournal>("create_stored_journal", {
    name,
    defaultCurrency: defaultCurrency ?? null,
  });
}

/** Copy a just-picked file (CSV/rules) to a stable cache path that outlives
 *  the iOS picker Inbox. */
export async function stashPickedFile(path: string): Promise<string> {
  return invoke<string>("stash_picked_file", { path });
}

// ─── CSV Import ───

export async function previewCsvImport(
  csvPath: string,
  rulesPath: string
): Promise<CsvPreview> {
  return invoke<CsvPreview>("preview_csv_import", { csvPath, rulesPath });
}

export async function importCsv(
  csvPath: string,
  rulesPath: string,
  selectedIndices: number[]
): Promise<CsvImportResult> {
  return invoke<CsvImportResult>("import_csv", {
    csvPath,
    rulesPath,
    selectedIndices,
  });
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

export async function finishReconciliation(
  force?: boolean
): Promise<JournalSummary> {
  return invoke<JournalSummary>("finish_reconciliation", {
    force: force ?? null,
  });
}

// ─── Valuation ───

export interface ValuationInfo {
  targetCommodity: string;
  unconvertible: string[];
}

export async function valuationInfo(
  params: ReportParams = {}
): Promise<ValuationInfo> {
  return invoke<ValuationInfo>("valuation_info", { params });
}

export async function cancelReconciliation(): Promise<void> {
  return invoke<void>("cancel_reconciliation");
}

// ─── Forecast ───

export async function getForecastRules(): Promise<ForecastRule[]> {
  return invoke<ForecastRule[]>("get_forecast_rules");
}

export async function saveForecastRule(
  period: string,
  description: string,
  postings: SaveForecastPosting[],
  replaceLine?: number | null
): Promise<JournalSummary> {
  return invoke<JournalSummary>("save_forecast_rule", {
    period,
    description,
    postings,
    replaceLine: replaceLine ?? null,
  });
}

export async function deleteForecastRule(
  line: number
): Promise<JournalSummary> {
  return invoke<JournalSummary>("delete_forecast_rule", { line });
}

export async function forecastProjection(
  account: string | null,
  horizon: string | null,
  params: ReportParams = {}
): Promise<ForecastProjection> {
  return invoke<ForecastProjection>("forecast_projection", {
    account,
    horizon,
    params,
  });
}

export async function upcomingTransactions(
  horizon: string | null,
  limit?: number | null
): Promise<TransactionSummary[]> {
  return invoke<TransactionSummary[]>("upcoming_transactions", {
    horizon,
    limit: limit ?? null,
  });
}
