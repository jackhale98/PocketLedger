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
  /** Carries costs/assertions/tags the form doesn't show; edits preserve them
   *  as long as the posting structure is unchanged */
  hasHiddenDetails: boolean;
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
  /** hledger query expression (acct:, amt:, date:, cur:, status:, tag:, not:, ...) */
  query?: string | null;
  /** Include forecast transactions generated from periodic rules */
  forecast?: boolean | null;
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

/** Interval for a multi-period balance report */
export type BalanceInterval = "weekly" | "monthly" | "quarterly" | "yearly";

/** Accumulation mode for a multi-period balance report */
export type BalanceAccumulationMode = "periodic" | "cumulative" | "historical";

/** A row in a multi-period balance report */
export interface PeriodicBalanceRow {
  account: string;
  depth: number;
  /** One cell per period, in period order; empty array = zero */
  amounts: AmountEntry[][];
  /** Row total (periodic/cumulative: sum of changes; historical: final balance) */
  total: AmountEntry[];
}

/** Multi-period balance report */
export interface PeriodicBalanceReport {
  /** Period labels, e.g. "2024-01" / "2024-Q1" / "2024" / "2024-W05" */
  periods: string[];
  rows: PeriodicBalanceRow[];
  /** Column totals across all rows, one per period */
  totals: AmountEntry[][];
}

/** A row in a register report */
export interface RegisterRow {
  date: string;
  description: string;
  account: string;
  amount: AmountEntry[];
  runningTotal: AmountEntry[];
  /** Source transaction index, for opening the editor. null when the row has
   *  no editable source (forecast projections, auto-generated postings). */
  transactionIndex: number | null;
  /** Projected or auto-generated rather than present in the journal file. */
  generated: boolean;
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
  /** Income-style goal (negative budget): overBudget means the goal was missed */
  isIncome: boolean;
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
  /** Full period expression, e.g. "monthly" or "every 2 weeks from 2026-01" */
  period: string;
  description: string;
  /** Source line of the periodic transaction; use for replace/delete */
  line: number;
  entries: BudgetEntry[];
}

/** Platform + storage info (mobile journals live in the app's documents dir) */
export interface PlatformInfo {
  isMobile: boolean;
  storageDir: string;
}

/** A journal file in the app's storage directory */
export interface StoredJournal {
  name: string;
  path: string;
  /** Unix seconds; 0 if unknown */
  modified: number;
  size: number;
}

/** Result of importing an external journal into app storage */
export interface ImportedJournal {
  path: string;
  fileName: string;
  /** Identical copy already stored; reused as-is */
  reused: boolean;
  /** Name conflict with different content; imported under a numbered name */
  renamed: boolean;
}

/** Result of creating a journal in app storage */
export interface CreatedJournal {
  path: string;
  fileName: string;
  summary: JournalSummary;
}

/** CSV import preview transaction */
export interface CsvPreviewTransaction {
  date: string;
  description: string;
  account1: string;
  account2: string;
  amount: string;
  commodity: string;
  comment: string | null;
  /** A matching transaction (date+amount+description) already exists */
  isDuplicate: boolean;
}

/** CSV import preview result */
export interface CsvPreview {
  transactions: CsvPreviewTransaction[];
  warnings: string[];
  rowsProcessed: number;
  duplicateCount: number;
}

/** CSV import result */
export interface CsvImportResult {
  importedCount: number;
  skippedDuplicates: number;
  warnings: string[];
  summary: JournalSummary;
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

/** A posting of a periodic (~) rule as read from the journal */
export interface ForecastPosting {
  account: string;
  /** null = elided; hledger infers it to balance the rule */
  amount: string | null;
  commodity: string;
}

/** A periodic (~) rule in the journal */
export interface ForecastRule {
  /** Full period expression, e.g. "monthly from 2026-01" */
  period: string;
  description: string;
  /** Source line of the rule; use for replace/delete */
  line: number;
  postings: ForecastPosting[];
  /** Set when the period expression can't be honored; the rule generates nothing */
  error: string | null;
}

/** Posting input for creating/editing a periodic rule */
export interface SaveForecastPosting {
  account: string;
  amount: string | null;
  commodity: string | null;
}

/** One month of a cash-flow projection */
export interface ProjectionPoint {
  /** "YYYY-MM" */
  period: string;
  inflow: string;
  /** Money out, positive */
  outflow: string;
  closing: string;
  /** false = recorded activity, true = projected */
  projected: boolean;
}

/** A projected cash-flow shortfall */
export interface ShortfallAlert {
  date: string;
  balance: string;
  description: string;
}

/** Cash-flow projection for an account tree */
export interface ForecastProjection {
  points: ProjectionPoint[];
  shortfall: ShortfallAlert | null;
  lastActual: string | null;
  horizon: string | null;
  commodity: string;
  noRules: boolean;
}
