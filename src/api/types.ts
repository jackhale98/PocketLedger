/** Mirrors the Rust JournalSummary struct */
export interface JournalSummary {
  fileName: string;
  transactionCount: number;
  accountCount: number;
  warnings: string[];
}

/** Mirrors the Rust TransactionSummary struct */
export interface TransactionSummary {
  index: number;
  date: string;
  status: string;
  description: string;
  comment: string | null;
  postings: PostingSummary[];
}

/** Mirrors the Rust PostingSummary struct */
export interface PostingSummary {
  account: string;
  amount: string | null;
  commodity: string | null;
  comment: string | null;
}

/** Input for creating a new transaction */
export interface NewTransaction {
  date: string;
  status: string;
  description: string;
  comment: string | null;
  postings: NewPosting[];
}

/** Input for a posting in a new transaction */
export interface NewPosting {
  account: string;
  amount: string | null;
  commodity: string | null;
  comment: string | null;
}

/** Report query parameters */
export interface ReportParams {
  dateFrom?: string | null;
  dateTo?: string | null;
  accountFilter?: string | null;
  targetCommodity?: string | null;
}

/** A row in a balance report */
export interface BalanceRow {
  account: string;
  depth: number;
  amounts: AmountEntry[];
}

/** A single amount entry */
export interface AmountEntry {
  commodity: string;
  quantity: string;
}

/** A row in a register report */
export interface RegisterRow {
  date: string;
  description: string;
  account: string;
  amount: AmountEntry[];
  runningTotal: AmountEntry[];
}

/** Time series data point */
export interface TimeSeriesPoint {
  date: string;
  value: string;
}

/** Income vs Expense data point */
export interface IncomeExpensePoint {
  period: string;
  income: string;
  expenses: string;
}

/** Pie chart slice */
export interface PieSlice {
  name: string;
  value: string;
}

/** Section of a financial statement */
export interface StatementSection {
  title: string;
  rows: BalanceRow[];
  total: AmountEntry[];
}

/** Full financial statement */
export interface FinancialStatement {
  title: string;
  sections: StatementSection[];
  net: AmountEntry[];
}

/** Budget comparison row */
export interface BudgetRow {
  account: string;
  budget: string;
  actual: string;
  difference: string;
  percentage: string;
  commodity: string;
  overBudget: boolean;
}

/** Budget vs actual chart data point */
export interface BudgetSummaryPoint {
  period: string;
  budgeted: string;
  actual: string;
}

/** Budget entry for creating/editing */
export interface BudgetEntry {
  account: string;
  amount: string;
  commodity: string;
}

/** Budget info from journal */
export interface BudgetInfo {
  period: string;
  entries: BudgetEntry[];
}

/** Reconciliation posting */
export interface ReconciliationPosting {
  transactionIndex: number;
  postingIndex: number;
  date: string;
  description: string;
  amount: string;
  commodity: string;
  isCleared: boolean;
}

/** Reconciliation session state */
export interface ReconciliationState {
  account: string;
  statementDate: string;
  statementBalance: string;
  statementCommodity: string;
  clearedBalance: string;
  difference: string;
  isReconciled: boolean;
  postings: ReconciliationPosting[];
}
