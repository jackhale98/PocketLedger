/** Mirrors the Rust JournalSummary struct */
export interface JournalSummary {
  fileName: string;
  transactionCount: number;
  accountCount: number;
  warnings: string[];
  /** `include` targets that couldn't be resolved, as written in the journal.
   *  On mobile these are usually siblings that were never imported. */
  missingIncludes: string[];
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

/** A file participating in the loaded journal */
export interface JournalFileInfo {
  index: number;
  name: string;
  path: string;
  /** The file that was opened; the rest arrived via `include` */
  isMain: boolean;
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
  /** Period labels, matching hledger: "2024-01" / "2024Q1" / "2024" / "2024-W05" */
  periods: string[];
  rows: PeriodicBalanceRow[];
  /** Column totals across all rows, one per period */
  totals: AmountEntry[][];
}

/** One point in a commodity's price history */
export interface PricePoint {
  date: string;
  rate: string;
}

/** A commodity pair and its price over time */
export interface PriceSeries {
  base: string;
  quote: string;
  points: PricePoint[];
}

/** How often an account is posted to */
export interface AccountActivity {
  account: string;
  postings: number;
  lastSeen: string;
}

/** Postings in one month */
export interface ActivityPoint {
  period: string;
  postings: number;
}

/** Summary facts about the journal */
export interface JournalStatistics {
  transactionCount: number;
  postingCount: number;
  /** Accounts actually posted to, not the full tree including parents */
  accountCount: number;
  commodities: string[];
  firstDate: string | null;
  lastDate: string | null;
  daysCovered: number;
  /** Mean transactions per month over the covered span */
  perMonth: string;
  busiestAccounts: AccountActivity[];
  activity: ActivityPoint[];
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
  /** Window this row's actual was summed over — the report range clipped to
   *  the budget's own period */
  periodFrom: string;
  periodTo: string;
}

/** A budget whose period yields no goal in the reported range */
export interface InactiveBudget {
  line: number;
  /** Period expression as written */
  period: string;
  description: string;
  accounts: string[];
  /** The rule's own start, when it has one */
  starts: string | null;
  /** The rule's own last covered day, when it has one */
  ends: string | null;
}

/** Budget goals vs actuals, plus budgets outside the reported range */
/** Per-commodity totals, computed from postings — summing rows double-counts
 *  nested budgets. */
export interface BudgetTotal {
  commodity: string;
  budget: string;
  actual: string;
  /** Budget less actual, subtracted in exact decimal arithmetic. */
  remaining: string;
}

export interface BudgetComparison {
  rows: BudgetRow[];
  totals: BudgetTotal[];
  inactive: InactiveBudget[];
  /** The range actually reported on; "all time" means the journal's own span */
  from: string;
  to: string;
}

/** Budget vs actual chart data point */
export interface BudgetSummaryPoint {
  period: string;
  budgeted: string;
  actual: string;
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
  /** Rules that generated nothing, with the reason. */
  ruleErrors: string[];
  /** Days between the last recorded transaction and today; large values mean
   *  the projection starts from a balance that hasn't been updated lately. */
  daysSinceLastActual: number | null;
}

export interface RoiCashFlow {
  date: string;
  /** Positive when money enters the investment. */
  amount: string;
}

export interface RoiReport {
  begin: string;
  end: string;
  valueBegin: string;
  valueEnd: string;
  /** Net money added over the period. */
  cashflow: string;
  /** Value gained beyond what was contributed. */
  pnl: string;
  /** Percentages, already annualised. Null when unsolvable. */
  irr: string | null;
  twrPeriod: string | null;
  twrAnnual: string | null;
  commodity: string;
  /** What the accounts hold, in their own units. */
  heldCommodities: string[];
  flows: RoiCashFlow[];
}
