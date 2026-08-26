use std::collections::{BTreeMap, BTreeSet};

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::amount::MixedAmount;
use crate::balance::ResolvedTransaction;
use crate::classify::{AccountClassifier, AccountType};
use crate::price_db::PriceDb;

/// A row in a balance report.
#[derive(Debug, Clone, Serialize)]
pub struct BalanceRow {
    pub account: String,
    pub depth: usize,
    pub amounts: Vec<AmountEntry>,
}

/// A single commodity amount for serialization.
#[derive(Debug, Clone, Serialize)]
pub struct AmountEntry {
    pub commodity: String,
    pub quantity: String,
}

/// A row in a register report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRow {
    pub date: String,
    pub description: String,
    pub account: String,
    pub amount: Vec<AmountEntry>,
    pub running_total: Vec<AmountEntry>,
    /// Index of the source transaction, for opening it in the editor. None
    /// for rows that have no editable source: forecast projections and
    /// auto-generated postings exist only in memory.
    pub transaction_index: Option<usize>,
    /// Forecast or auto-posting output rather than something in the file.
    pub generated: bool,
}

/// A data point for time-series charts.
#[derive(Debug, Clone, Serialize)]
pub struct TimeSeriesPoint {
    pub date: String,
    pub value: String,
}

/// Income vs Expense data for a single period.
#[derive(Debug, Clone, Serialize)]
pub struct IncomeExpensePoint {
    pub period: String,
    pub income: String,
    pub expenses: String,
}

/// A slice of a pie chart.
#[derive(Debug, Clone, Serialize)]
pub struct PieSlice {
    pub name: String,
    pub value: String,
}

/// A section of a financial statement (e.g. Assets, Liabilities).
#[derive(Debug, Clone, Serialize)]
pub struct StatementSection {
    pub title: String,
    pub rows: Vec<BalanceRow>,
    pub total: Vec<AmountEntry>,
}

/// A complete financial statement (Balance Sheet, Income Statement, etc.)
#[derive(Debug, Clone, Serialize)]
pub struct FinancialStatement {
    pub title: String,
    pub sections: Vec<StatementSection>,
    pub net: Vec<AmountEntry>,
}

fn mixed_to_entries(m: &MixedAmount) -> Vec<AmountEntry> {
    if m.amounts.is_empty() {
        return vec![AmountEntry {
            commodity: String::new(),
            quantity: "0".to_string(),
        }];
    }
    m.amounts
        .iter()
        .map(|(c, q)| AmountEntry {
            commodity: c.clone(),
            quantity: q.normalize().to_string(),
        })
        .collect()
}

/// Convert a MixedAmount to a target commodity using the price database.
/// Commodities without any conversion path are KEPT in their own commodity
/// (hledger -V behavior) — never silently dropped, never raw-summed.
pub fn convert_mixed(
    m: &MixedAmount,
    target: &str,
    price_db: &PriceDb,
    date: NaiveDate,
) -> MixedAmount {
    let mut result = MixedAmount::zero();
    for (commodity, quantity) in &m.amounts {
        if commodity == target {
            result.add(target, *quantity);
        } else if let Some(converted) = price_db.convert(*quantity, commodity, target, date) {
            result.add(target, converted);
        } else {
            result.add(commodity, *quantity);
        }
    }
    result
}

/// Value a MixedAmount in the target commodity for a single-number chart.
/// Returns the convertible total; commodities with no conversion path are
/// EXCLUDED from the number (never added raw across commodities) and reported
/// in the second element so the UI can warn.
pub fn valued_quantity(
    m: &MixedAmount,
    target: &str,
    price_db: &PriceDb,
    date: NaiveDate,
) -> (Decimal, BTreeSet<String>) {
    let mut total = Decimal::ZERO;
    let mut unconvertible = BTreeSet::new();
    for (commodity, quantity) in &m.amounts {
        if commodity == target || commodity.is_empty() {
            total += *quantity;
        } else if let Some(converted) = price_db.convert(*quantity, commodity, target, date) {
            total += converted;
        } else {
            unconvertible.insert(commodity.clone());
        }
    }
    (total, unconvertible)
}

/// Commodities in the journal that cannot be valued in `target` as of `date`.
/// The UI shows these as a warning on valued charts.
pub fn unconvertible_commodities(
    transactions: &[ResolvedTransaction],
    target: &str,
    price_db: &PriceDb,
    date: NaiveDate,
) -> Vec<String> {
    let mut result = BTreeSet::new();
    for txn in transactions {
        for posting in &txn.postings {
            for commodity in posting.amount.amounts.keys() {
                if commodity != target
                    && !commodity.is_empty()
                    && price_db
                        .convert(Decimal::ONE, commodity, target, date)
                        .is_none()
                {
                    result.insert(commodity.clone());
                }
            }
        }
    }
    result.into_iter().collect()
}

/// Case-insensitive account-prefix match with a `:` boundary.
pub fn account_matches_prefix(account: &str, prefix: &str) -> bool {
    if account.len() < prefix.len() {
        return false;
    }
    let account_lower = account.to_lowercase();
    let prefix_lower = prefix.to_lowercase();
    account_lower == prefix_lower
        || (account_lower.starts_with(&prefix_lower)
            && account_lower.as_bytes().get(prefix_lower.len()) == Some(&b':'))
}

// ─── Report generation functions ───

/// Generate a balance report: account balances filtered by account prefix and
/// date range. Dates are matched per-posting (respecting `date:` tags).
pub fn balance_report(
    transactions: &[ResolvedTransaction],
    account_filter: Option<&str>,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Vec<BalanceRow> {
    let mut balances: BTreeMap<String, MixedAmount> = BTreeMap::new();

    for txn in transactions {
        for posting in &txn.postings {
            if !date_in_range(posting.date, date_from, date_to) {
                continue;
            }
            if let Some(filter) = account_filter {
                if !account_matches_prefix(&posting.account.full, filter) {
                    continue;
                }
            }
            let entry = balances
                .entry(posting.account.full.clone())
                .or_insert_with(MixedAmount::zero);
            entry.add_mixed(&posting.amount);
        }
    }

    rows_with_parents(balances)
}

fn date_in_range(date: NaiveDate, from: Option<NaiveDate>, to: Option<NaiveDate>) -> bool {
    if let Some(f) = from {
        if date < f {
            return false;
        }
    }
    if let Some(t) = to {
        if date > t {
            return false;
        }
    }
    true
}

/// Add parent accounts, compute inclusive balances, drop zero rows.
fn rows_with_parents(balances: BTreeMap<String, MixedAmount>) -> Vec<BalanceRow> {
    let mut balances = balances;
    let leaf_accounts: Vec<String> = balances.keys().cloned().collect();
    for account in &leaf_accounts {
        let parts: Vec<&str> = account.split(':').collect();
        for depth in 1..parts.len() {
            let parent = parts[..depth].join(":");
            balances.entry(parent).or_insert_with(MixedAmount::zero);
        }
    }

    let all_accounts: Vec<String> = balances.keys().cloned().collect();
    let mut inclusive: BTreeMap<String, MixedAmount> = BTreeMap::new();

    for account in &all_accounts {
        let mut total = balances.get(account).cloned().unwrap_or_default();
        for (other, amt) in &balances {
            if other != account
                && other.starts_with(account.as_str())
                && other.as_bytes().get(account.len()) == Some(&b':')
            {
                total.add_mixed(amt);
            }
        }
        inclusive.insert(account.clone(), total);
    }

    inclusive
        .iter()
        .filter(|(_, amt)| !amt.is_zero())
        .map(|(account, amt)| {
            let depth = account.matches(':').count();
            BalanceRow {
                account: account.clone(),
                depth,
                amounts: mixed_to_entries(amt),
            }
        })
        .collect()
}

/// Generate a balance report with values converted to a target commodity using
/// market prices. Unpriceable commodities stay in their own commodity.
pub fn balance_report_valued(
    transactions: &[ResolvedTransaction],
    account_filter: Option<&str>,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    target_commodity: &str,
    price_db: &PriceDb,
) -> Vec<BalanceRow> {
    // Today, not the last transaction: a journal that hasn't been touched in
    // months should still be valued at the newest price available, which is
    // what `hledger -V` does.
    let valuation_date = date_to.unwrap_or_else(|| chrono::Local::now().date_naive());

    let mut balances: BTreeMap<String, MixedAmount> = BTreeMap::new();

    for txn in transactions {
        for posting in &txn.postings {
            if !date_in_range(posting.date, date_from, date_to) {
                continue;
            }
            if let Some(filter) = account_filter {
                if !account_matches_prefix(&posting.account.full, filter) {
                    continue;
                }
            }
            let entry = balances
                .entry(posting.account.full.clone())
                .or_insert_with(MixedAmount::zero);
            entry.add_mixed(&posting.amount);
        }
    }

    // Value the inclusive totals.
    let rows = rows_with_parents(balances);
    rows.into_iter()
        .map(|row| {
            let mut m = MixedAmount::zero();
            for e in &row.amounts {
                if let Ok(q) = e.quantity.parse::<Decimal>() {
                    m.add(&e.commodity, q);
                }
            }
            let valued = convert_mixed(&m, target_commodity, price_db, valuation_date);
            BalanceRow {
                account: row.account,
                depth: row.depth,
                amounts: mixed_to_entries(&valued),
            }
        })
        .collect()
}

/// Generate a register report for a specific account (boundary-aware prefix:
/// `assets:bank` matches `assets:bank` and `assets:bank:x`, not `assets:bankloan`).
pub fn register_report(
    transactions: &[ResolvedTransaction],
    account_filter: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Vec<RegisterRow> {
    let mut rows = Vec::new();
    let mut running_total = MixedAmount::zero();

    for txn in transactions {
        for posting in &txn.postings {
            if !account_matches_prefix(&posting.account.full, account_filter) {
                continue;
            }
            if !date_in_range(posting.date, date_from, date_to) {
                continue;
            }
            running_total.add_mixed(&posting.amount);
            rows.push(RegisterRow {
                date: posting.date.format("%Y-%m-%d").to_string(),
                description: txn.description.clone(),
                account: posting.account.full.clone(),
                amount: mixed_to_entries(&posting.amount),
                running_total: mixed_to_entries(&running_total),
                transaction_index: (!posting.generated).then_some(posting.transaction_index),
                generated: posting.generated,
            });
        }
    }

    rows
}

/// Generate a Balance Sheet. Historical semantics like hledger `bs`: shows
/// balances as of `date_to`; a from-date filter deliberately does NOT truncate
/// opening balances (that would turn balances into period deltas).
pub fn balance_sheet(
    transactions: &[ResolvedTransaction],
    classifier: &AccountClassifier,
    price_db: &PriceDb,
    target: &str,
    _date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> FinancialStatement {
    let on = valuation_date(transactions, date_to);
    let value = |d| value_section(d, target, price_db, on);
    let assets = value(section_by_type(transactions, classifier, &[AccountType::Asset, AccountType::Cash], None, date_to));
    let liabilities = value(section_by_type(transactions, classifier, &[AccountType::Liability], None, date_to));
    let equity = value(section_by_type(transactions, classifier, &[AccountType::Equity], None, date_to));

    // Net worth = assets + liabilities (liabilities are negative).
    let mut net = assets.total.clone();
    net.add_mixed(&liabilities.total);

    FinancialStatement {
        title: "Balance Sheet".to_string(),
        sections: vec![
            format_section("Assets", &assets),
            // Net below still uses the raw signs, so net worth is unaffected.
            negated_section("Liabilities", &liabilities),
            negated_section("Equity", &equity),
        ],
        net: mixed_to_entries(&net),
    }
}

/// Generate an Income Statement (Revenue - Expenses = Net Income) for a period.
pub fn income_statement(
    transactions: &[ResolvedTransaction],
    classifier: &AccountClassifier,
    price_db: &PriceDb,
    target: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> FinancialStatement {
    let on = valuation_date(transactions, date_to);
    let value = |d| value_section(d, target, price_db, on);
    let income = value(section_by_type(transactions, classifier, &[AccountType::Revenue], date_from, date_to));
    let expenses = value(section_by_type(transactions, classifier, &[AccountType::Expense], date_from, date_to));

    let income_negated = income.total.negate(); // income is negative in double-entry
    let mut net = income_negated.clone();
    net.subtract(&expenses.total);

    FinancialStatement {
        title: "Income Statement".to_string(),
        sections: vec![
            StatementSection {
                title: "Income".to_string(),
                rows: negated_rows(&income.rows),
                total: mixed_to_entries(&income_negated),
            },
            format_section("Expenses", &expenses),
        ],
        net: mixed_to_entries(&net),
    }
}

/// Generate a Cash Flow statement: changes in cash accounts over the period.
pub fn cash_flow(
    transactions: &[ResolvedTransaction],
    classifier: &AccountClassifier,
    price_db: &PriceDb,
    target: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> FinancialStatement {
    let mut balances: BTreeMap<String, MixedAmount> = BTreeMap::new();
    let mut total = MixedAmount::zero();

    for txn in transactions {
        for posting in &txn.postings {
            if !date_in_range(posting.date, date_from, date_to) {
                continue;
            }
            if !classifier.is_cash(&posting.account.full) {
                continue;
            }
            balances
                .entry(posting.account.full.clone())
                .or_insert_with(MixedAmount::zero)
                .add_mixed(&posting.amount);
            total.add_mixed(&posting.amount);
        }
    }

    let valued = value_section(
        SectionData { rows: rows_with_parents(balances), total },
        target,
        price_db,
        valuation_date(transactions, date_to),
    );

    FinancialStatement {
        title: "Cash Flow".to_string(),
        sections: vec![StatementSection {
            title: "Cash Changes".to_string(),
            rows: valued.rows,
            total: mixed_to_entries(&valued.total),
        }],
        net: mixed_to_entries(&valued.total),
    }
}

/// Net worth over time: assets + liabilities valued in the target commodity at
/// the end of each month (market prices; unpriceable holdings excluded from
/// the number — see `unconvertible_commodities` for the warning list).
pub fn net_worth_series(
    transactions: &[ResolvedTransaction],
    classifier: &AccountClassifier,
    price_db: &PriceDb,
    target_commodity: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    // Pin the bucket size; None picks one from the range.
    step: Option<SeriesStep>,
) -> Vec<TimeSeriesPoint> {
    if transactions.is_empty() {
        return vec![];
    }

    let first_date = date_from.unwrap_or(transactions.first().unwrap().date);
    let last_date = date_to.unwrap_or(transactions.last().unwrap().date);

    let mut points = Vec::new();
    let mut balance = MixedAmount::zero();
    let mut txn_idx = 0;

    let step = step.unwrap_or_else(|| series_step(first_date, last_date));
    let mut current = period_end(first_date, step);
    while current <= period_end(last_date, step) {
        while txn_idx < transactions.len() && transactions[txn_idx].date <= current {
            for posting in &transactions[txn_idx].postings {
                let t = classifier.classify(&posting.account.full);
                if t.is_asset() || t == AccountType::Liability {
                    balance.add_mixed(&posting.amount);
                }
            }
            txn_idx += 1;
        }

        let (net_worth, _skipped) =
            valued_quantity(&balance, target_commodity, price_db, current);

        points.push(TimeSeriesPoint {
            date: current.format("%Y-%m-%d").to_string(),
            value: net_worth.normalize().to_string(),
        });

        current = next_period_end(current, step);
    }

    points
}

/// Account balance over time for a specific account, valued in the target.
pub fn account_series(
    transactions: &[ResolvedTransaction],
    price_db: &PriceDb,
    account_prefix: &str,
    target_commodity: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    // Pin the bucket size; None picks one from the range.
    step: Option<SeriesStep>,
) -> Vec<TimeSeriesPoint> {
    if transactions.is_empty() {
        return vec![];
    }

    let first_date = date_from.unwrap_or(transactions.first().unwrap().date);
    let last_date = date_to.unwrap_or(transactions.last().unwrap().date);

    let mut points = Vec::new();
    let mut balance = MixedAmount::zero();
    let mut txn_idx = 0;

    let step = step.unwrap_or_else(|| series_step(first_date, last_date));
    let mut current = period_end(first_date, step);
    while current <= period_end(last_date, step) {
        while txn_idx < transactions.len() && transactions[txn_idx].date <= current {
            for posting in &transactions[txn_idx].postings {
                if account_matches_prefix(&posting.account.full, account_prefix) {
                    balance.add_mixed(&posting.amount);
                }
            }
            txn_idx += 1;
        }

        let (value, _skipped) = valued_quantity(&balance, target_commodity, price_db, current);
        points.push(TimeSeriesPoint {
            date: current.format("%Y-%m-%d").to_string(),
            value: value.normalize().to_string(),
        });

        current = next_period_end(current, step);
    }

    points
}

/// Income vs Expenses by month. Values are converted to the target commodity
/// at each period end; the requested date range is respected exactly (the
/// first bucket does not swallow days before `date_from`). Signs are true:
/// a refund-heavy month shows negative expenses rather than being folded to
/// positive by abs().
pub fn income_expense_series(
    transactions: &[ResolvedTransaction],
    classifier: &AccountClassifier,
    price_db: &PriceDb,
    target_commodity: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    // Pin the bucket size; None picks one from the range.
    step: Option<SeriesStep>,
) -> Vec<IncomeExpensePoint> {
    if transactions.is_empty() {
        return vec![];
    }

    let first_date = date_from.unwrap_or(transactions.first().unwrap().date);
    let last_date = date_to.unwrap_or(transactions.last().unwrap().date);

    let mut points = Vec::new();

    let step = step.unwrap_or_else(|| series_step(first_date, last_date));
    let mut current_start = period_start(first_date, step);
    while current_start <= last_date {
        let current_end = period_end(current_start, step);
        // Clamp the bucket to the requested range.
        let bucket_from = current_start.max(first_date);
        let bucket_to = current_end.min(last_date);

        let mut income = MixedAmount::zero();
        let mut expenses = MixedAmount::zero();

        for txn in transactions {
            for posting in &txn.postings {
                if !date_in_range(posting.date, Some(bucket_from), Some(bucket_to)) {
                    continue;
                }
                match classifier.classify(&posting.account.full) {
                    AccountType::Revenue => income.add_mixed(&posting.amount),
                    AccountType::Expense => expenses.add_mixed(&posting.amount),
                    _ => {}
                }
            }
        }

        let (income_sum, _) = valued_quantity(&income, target_commodity, price_db, current_end);
        let (expense_sum, _) =
            valued_quantity(&expenses, target_commodity, price_db, current_end);

        points.push(IncomeExpensePoint {
            period: period_label(current_start, step),
            // Income is negative in double-entry; flip for display.
            income: (-income_sum).normalize().to_string(),
            // Expenses shown as negative bars (spending) / positive (refunds).
            expenses: (-expense_sum).normalize().to_string(),
        });

        current_start = next_period_start(current_start, step);
    }

    points
}

/// Expense breakdown by subcategory, with optional drill-down via parent_prefix.
pub fn expense_breakdown(
    transactions: &[ResolvedTransaction],
    price_db: &PriceDb,
    target_commodity: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    parent_prefix: Option<&str>,
) -> Vec<PieSlice> {
    let prefix = parent_prefix.unwrap_or("expenses");
    let prefix_depth = prefix.matches(':').count() + 1; // depth of children

    // Today, not the last transaction: a journal that hasn't been touched in
    // months should still be valued at the newest price available, which is
    // what `hledger -V` does.
    let valuation_date = date_to.unwrap_or_else(|| chrono::Local::now().date_naive());

    let mut by_category: BTreeMap<String, Decimal> = BTreeMap::new();

    for txn in transactions {
        for posting in &txn.postings {
            if !date_in_range(posting.date, date_from, date_to) {
                continue;
            }
            if !account_matches_prefix(&posting.account.full, prefix) {
                continue;
            }

            let category = posting
                .account
                .parts
                .get(prefix_depth)
                .cloned()
                .unwrap_or_else(|| "other".to_string());

            let (value, _) = valued_quantity(
                &posting.amount,
                target_commodity,
                price_db,
                valuation_date,
            );
            *by_category.entry(category).or_insert(Decimal::ZERO) += value;
        }
    }

    // Sort by value descending, keep top 7, group rest as "other".
    let mut sorted: Vec<(String, Decimal)> = by_category
        .into_iter()
        .filter(|(_, v)| *v > Decimal::ZERO)
        .collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    const MAX_SLICES: usize = 7;
    if sorted.len() > MAX_SLICES {
        let top = &sorted[..MAX_SLICES];
        let other_total: Decimal = sorted[MAX_SLICES..].iter().map(|(_, v)| v).sum();
        let mut result: Vec<PieSlice> = top
            .iter()
            .map(|(name, value)| PieSlice {
                name: name.clone(),
                value: value.normalize().to_string(),
            })
            .collect();
        if !other_total.is_zero() {
            result.push(PieSlice {
                name: "other".to_string(),
                value: other_total.normalize().to_string(),
            });
        }
        result
    } else {
        sorted
            .into_iter()
            .map(|(name, value)| PieSlice {
                name,
                value: value.normalize().to_string(),
            })
            .collect()
    }
}

// ─── Helper types ───

struct SectionData {
    rows: Vec<BalanceRow>,
    total: MixedAmount,
}

/// Value a section's rows and total, keeping commodities that have no
/// conversion path in their own units rather than dropping them.
/// Prices are looked up as of the report's end date, falling back to the last
/// transaction so a report with no explicit end still uses current prices.
fn valuation_date(_transactions: &[ResolvedTransaction], date_to: Option<NaiveDate>) -> NaiveDate {
    date_to.unwrap_or_else(|| chrono::Local::now().date_naive())
}

fn value_section(data: SectionData, target: &str, price_db: &PriceDb, date: NaiveDate) -> SectionData {
    if target.is_empty() {
        return data;
    }
    SectionData {
        rows: data
            .rows
            .into_iter()
            .map(|row| {
                let mut m = MixedAmount::zero();
                for entry in &row.amounts {
                    if let Ok(q) = Decimal::from_str_exact(&entry.quantity) {
                        m.add(&entry.commodity, q);
                    }
                }
                BalanceRow {
                    amounts: mixed_to_entries(&convert_mixed(&m, target, price_db, date)),
                    ..row
                }
            })
            .collect(),
        total: convert_mixed(&data.total, target, price_db, date),
    }
}

fn section_by_type(
    transactions: &[ResolvedTransaction],
    classifier: &AccountClassifier,
    types: &[AccountType],
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> SectionData {
    let mut balances: BTreeMap<String, MixedAmount> = BTreeMap::new();
    let mut total = MixedAmount::zero();

    for txn in transactions {
        for posting in &txn.postings {
            if !date_in_range(posting.date, date_from, date_to) {
                continue;
            }
            let t = classifier.classify(&posting.account.full);
            if !types.contains(&t) {
                continue;
            }
            balances
                .entry(posting.account.full.clone())
                .or_insert_with(MixedAmount::zero)
                .add_mixed(&posting.amount);
            total.add_mixed(&posting.amount);
        }
    }

    SectionData {
        rows: rows_with_parents(balances),
        total,
    }
}

/// Flip the sign of every amount in a set of rows.
///
/// Revenue, liabilities and equity are credits, so they carry negative
/// balances in double entry. hledger's statements negate them for display —
/// income reads as what you earned, a card balance as what you owe — and a
/// section whose total was negated while its rows were not contradicts itself.
fn negated_rows(rows: &[BalanceRow]) -> Vec<BalanceRow> {
    rows.iter()
        .map(|row| BalanceRow {
            account: row.account.clone(),
            depth: row.depth,
            amounts: row
                .amounts
                .iter()
                .map(|a| AmountEntry {
                    commodity: a.commodity.clone(),
                    quantity: a
                        .quantity
                        .parse::<Decimal>()
                        .map(|q| (-q).to_string())
                        .unwrap_or_else(|_| a.quantity.clone()),
                })
                .collect(),
        })
        .collect()
}

/// A section shown with credit balances flipped, like hledger's statements.
fn negated_section(title: &str, data: &SectionData) -> StatementSection {
    StatementSection {
        title: title.to_string(),
        rows: negated_rows(&data.rows),
        total: mixed_to_entries(&data.total.negate()),
    }
}

fn format_section(title: &str, data: &SectionData) -> StatementSection {
    StatementSection {
        title: title.to_string(),
        rows: data.rows.clone(),
        total: mixed_to_entries(&data.total),
    }
}

// ─── Date helpers ───

/// How finely to bucket a time series. Charting a one-month range by month
/// yields a single point, which draws nothing useful — the step shrinks with
/// the range so a zoomed-in view still shows a shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesStep {
    Day,
    Week,
    Month,
}

/// Pick a step giving a readable number of points for the range: roughly a
/// month or less daily, up to six months weekly, longer spans monthly.
pub fn parse_series_step(name: &str) -> Option<SeriesStep> {
    match name {
        "daily" | "D" => Some(SeriesStep::Day),
        "weekly" | "W" => Some(SeriesStep::Week),
        "monthly" | "M" => Some(SeriesStep::Month),
        _ => None,
    }
}

pub fn series_step(from: NaiveDate, to: NaiveDate) -> SeriesStep {
    match (to - from).num_days() {
        d if d <= 45 => SeriesStep::Day,
        d if d <= 186 => SeriesStep::Week,
        _ => SeriesStep::Month,
    }
}

/// Last day of the period containing `date`. Weeks end on Sunday, matching
/// the ISO weeks used elsewhere.
fn period_end(date: NaiveDate, step: SeriesStep) -> NaiveDate {
    match step {
        SeriesStep::Day => date,
        SeriesStep::Week => {
            let from_monday = date.weekday().num_days_from_monday() as i64;
            date + chrono::Duration::days(6 - from_monday)
        }
        SeriesStep::Month => end_of_month(date),
    }
}

fn period_start(date: NaiveDate, step: SeriesStep) -> NaiveDate {
    match step {
        SeriesStep::Day => date,
        SeriesStep::Week => {
            date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64)
        }
        SeriesStep::Month => start_of_month(date),
    }
}

fn next_period_end(date: NaiveDate, step: SeriesStep) -> NaiveDate {
    match step {
        SeriesStep::Day => date.succ_opt().unwrap_or(date),
        SeriesStep::Week => date + chrono::Duration::days(7),
        SeriesStep::Month => next_month_end(date),
    }
}

fn next_period_start(date: NaiveDate, step: SeriesStep) -> NaiveDate {
    match step {
        SeriesStep::Day => date.succ_opt().unwrap_or(date),
        SeriesStep::Week => date + chrono::Duration::days(7),
        SeriesStep::Month => next_month_start(date),
    }
}

/// Axis label: a bare month reads fine for monthly buckets, but finer steps
/// need the day to be distinguishable.
fn period_label(date: NaiveDate, step: SeriesStep) -> String {
    match step {
        SeriesStep::Month => date.format("%Y-%m").to_string(),
        _ => date.format("%m-%d").to_string(),
    }
}

fn end_of_month(date: NaiveDate) -> NaiveDate {
    let (y, m) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1)
        .unwrap()
        .pred_opt()
        .unwrap()
}

fn start_of_month(date: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap()
}

fn next_month_end(date: NaiveDate) -> NaiveDate {
    let next_start = next_month_start(date);
    end_of_month(next_start)
}

fn next_month_start(date: NaiveDate) -> NaiveDate {
    let (y, m) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::balance::resolve_transactions;
    use hledger_parser::parse;
    use rust_decimal_macros::dec;

    fn resolve(input: &str) -> Vec<ResolvedTransaction> {
        let journal = parse(input).unwrap();
        resolve_transactions(&journal).unwrap()
    }

    fn classifier() -> AccountClassifier {
        AccountClassifier::default()
    }

    fn no_prices() -> PriceDb {
        PriceDb::new()
    }

    #[test]
    fn balance_report_simple() {
        let txns = resolve(
            "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking\n",
        );
        let report = balance_report(&txns, None, None, None);

        let food = report.iter().find(|r| r.account == "expenses:food").unwrap();
        assert_eq!(food.amounts[0].quantity, "50");

        let checking = report.iter().find(|r| r.account == "assets:checking").unwrap();
        assert_eq!(checking.amounts[0].quantity, "-50");
    }

    #[test]
    fn balance_report_filtered_by_account() {
        let txns = resolve(
            "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking\n",
        );
        let report = balance_report(&txns, Some("expenses"), None, None);

        assert!(report.iter().any(|r| r.account == "expenses:food"));
        assert!(!report.iter().any(|r| r.account == "assets:checking"));
    }

    #[test]
    fn balance_report_filtered_by_date() {
        let txns = resolve(
            "2024-01-10 A\n    expenses:food  $30\n    assets:checking\n\n\
             2024-01-20 B\n    expenses:food  $20\n    assets:checking\n",
        );
        let report = balance_report(
            &txns,
            Some("expenses"),
            Some(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()),
            None,
        );
        let food = report.iter().find(|r| r.account == "expenses:food").unwrap();
        assert_eq!(food.amounts[0].quantity, "20");
    }

    #[test]
    fn register_report_for_account() {
        let txns = resolve(
            "2024-01-10 A\n    expenses:food  $30\n    assets:checking\n\n\
             2024-01-20 B\n    expenses:food  $20\n    assets:checking\n",
        );
        let report = register_report(&txns, "expenses:food", None, None);

        assert_eq!(report.len(), 2);
        assert_eq!(report[0].amount[0].quantity, "30");
        assert_eq!(report[0].running_total[0].quantity, "30");
        assert_eq!(report[1].amount[0].quantity, "20");
        assert_eq!(report[1].running_total[0].quantity, "50");
    }

    #[test]
    fn register_prefix_has_boundary() {
        let txns = resolve(
            "2024-01-10 A\n    assets:bank  $30\n    equity\n\n\
             2024-01-20 B\n    assets:bankloan  $20\n    equity\n",
        );
        let report = register_report(&txns, "assets:bank", None, None);
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].account, "assets:bank");
    }

    #[test]
    fn balance_sheet_net_includes_liabilities() {
        let txns = resolve(
            "2024-01-01 Opening\n    assets:checking  $10000\n    equity:opening\n\n\
             2024-01-05 Loan\n    assets:checking  $4000\n    liabilities:loan\n",
        );
        let bs = balance_sheet(&txns, &classifier(), &no_prices(), "", None, None);

        assert_eq!(bs.sections.len(), 3);
        // Net = 14000 (assets) + -4000 (liabilities) = 10000
        let net_usd = bs.net.iter().find(|e| e.commodity == "$").unwrap();
        assert_eq!(net_usd.quantity, "10000");
    }

    #[test]
    fn balance_sheet_is_historical() {
        // A from-date must NOT exclude opening balances.
        let txns = resolve(
            "2024-01-01 Opening\n    assets:checking  $1000\n    equity:opening\n\n\
             2024-03-15 Spend\n    expenses:food  $50\n    assets:checking\n",
        );
        let bs = balance_sheet(
            &txns,
            &classifier(),
            &no_prices(),
            "",
            Some(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()),
            None,
        );
        let assets = &bs.sections[0];
        let total = assets.total.iter().find(|e| e.commodity == "$").unwrap();
        assert_eq!(total.quantity, "950");
    }

    #[test]
    fn balance_sheet_uses_declared_types() {
        let journal = parse(
            "account aktiva:bank  ; type:A\n\n2024-01-01 T\n    aktiva:bank  $100\n    eigenkapital:start  $-100\n",
        )
        .unwrap();
        let txns = resolve_transactions(&journal).unwrap();
        let c = AccountClassifier::from_journal(&journal);
        let bs = balance_sheet(&txns, &c, &no_prices(), "", None, None);
        let assets = &bs.sections[0];
        assert!(assets.rows.iter().any(|r| r.account == "aktiva:bank"));
    }

    #[test]
    fn statistics_summarise_the_journal() {
        let txns = resolve(concat!(
            "2024-01-15 Pay\n    assets:checking  $3000\n    income:salary\n\n",
            "2024-01-20 Grocery\n    expenses:food  $50\n    assets:checking\n\n",
            "2024-03-15 Grocery\n    expenses:food  $60\n    assets:checking\n",
        ));
        let st = journal_statistics(&txns);

        assert_eq!(st.transaction_count, 3);
        assert_eq!(st.posting_count, 6);
        // Accounts actually posted to: checking, salary, food.
        assert_eq!(st.account_count, 3);
        assert_eq!(st.first_date.as_deref(), Some("2024-01-15"));
        assert_eq!(st.last_date.as_deref(), Some("2024-03-15"));
        // checking appears in all three transactions.
        assert_eq!(st.busiest_accounts[0].account, "assets:checking");
        assert_eq!(st.busiest_accounts[0].postings, 3);
        assert_eq!(st.busiest_accounts[0].last_seen, "2024-03-15");
        // February had no activity, so it isn't a bucket.
        assert_eq!(
            st.activity.iter().map(|a| a.period.as_str()).collect::<Vec<_>>(),
            vec!["2024-01", "2024-03"]
        );
    }

    #[test]
    fn statistics_of_an_empty_journal_do_not_divide_by_zero() {
        let st = journal_statistics(&[]);
        assert_eq!(st.transaction_count, 0);
        assert_eq!(st.days_covered, 0);
        assert_eq!(st.per_month, "0.0");
        assert!(st.first_date.is_none());
    }

    #[test]
    fn balance_sheet_values_holdings_and_keeps_unpriceable_ones() {
        // A brokerage account holding tickers and cash rendered as several
        // separate amounts on one row, which is unreadable on a phone. With a
        // target currency the priced holdings fold into it; a commodity with
        // no price and no cost stays in its own units rather than vanishing.
        let input = concat!(
            "P 2024-02-01 GME $25.00\n\n",
            "2024-01-10 Buy GME\n",
            "    assets:brokerage  4 GME @ $20.00\n",
            "    assets:checking\n\n",
            "2024-01-11 Grant\n",
            "    assets:brokerage  3 XYZ\n",
            "    equity:opening   -3 XYZ\n\n",
            "2024-01-01 Seed\n",
            "    assets:checking  $1000.00\n",
            "    equity:opening\n",
        );
        let journal = parse(input).unwrap();
        let txns = resolve_transactions(&journal).unwrap();
        let db = PriceDb::from_journal(&journal);

        let bs = balance_sheet(
            &txns,
            &classifier(),
            &db,
            "$",
            None,
            Some(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()),
        );
        let brokerage = bs.sections[0]
            .rows
            .iter()
            .find(|r| r.account == "assets:brokerage")
            .unwrap();

        // GME at the 2024-02-01 price: 4 x $25 = $100.
        let usd = brokerage.amounts.iter().find(|a| a.commodity == "$").unwrap();
        assert_eq!(usd.quantity, "100");
        // XYZ was acquired without a cost and has no price directive.
        let xyz = brokerage.amounts.iter().find(|a| a.commodity == "XYZ").unwrap();
        assert_eq!(xyz.quantity, "3");
    }

    #[test]
    fn balance_sheet_without_a_target_currency_is_unchanged() {
        let input = concat!(
            "2024-01-10 Buy\n",
            "    assets:brokerage  4 GME @ $20.00\n",
            "    assets:checking\n",
        );
        let journal = parse(input).unwrap();
        let txns = resolve_transactions(&journal).unwrap();
        let db = PriceDb::from_journal(&journal);

        let bs = balance_sheet(&txns, &classifier(), &db, "", None, None);
        let brokerage = bs.sections[0]
            .rows
            .iter()
            .find(|r| r.account == "assets:brokerage")
            .unwrap();
        assert_eq!(brokerage.amounts.len(), 1);
        assert_eq!(brokerage.amounts[0].commodity, "GME");
    }

    #[test]
    fn income_statement_basic() {
        let txns = resolve(
            "2024-01-15 Paycheck\n    assets:checking  $3000\n    income:salary\n\n\
             2024-01-20 Grocery\n    expenses:food  $50\n    assets:checking\n",
        );
        let is = income_statement(&txns, &classifier(), &no_prices(), "", None, None);

        assert_eq!(is.title, "Income Statement");
        assert_eq!(is.sections.len(), 2);
        let net = is.net.iter().find(|e| e.commodity == "$").unwrap();
        assert_eq!(net.quantity, "2950");
    }

    #[test]
    fn cash_flow_only_cash_accounts() {
        let txns = resolve(
            "2024-01-15 Pay\n    assets:bank:checking  $3000\n    income:salary\n\n\
             2024-01-20 Invest\n    assets:investments:etf  10 VTI @ $200\n    assets:bank:checking  $-2000\n",
        );
        let cf = cash_flow(&txns, &classifier(), &no_prices(), "", None, None);
        // Only the checking account changes count: 3000 - 2000 = 1000.
        let net = cf.net.iter().find(|e| e.commodity == "$").unwrap();
        assert_eq!(net.quantity, "1000");
    }

    #[test]
    fn expense_breakdown_basic() {
        let txns = resolve(
            "2024-01-10 A\n    expenses:food  $50\n    assets:checking\n\n\
             2024-01-15 B\n    expenses:rent  $1000\n    assets:checking\n\n\
             2024-01-20 C\n    expenses:food  $30\n    assets:checking\n",
        );
        let breakdown = expense_breakdown(&txns, &no_prices(), "$", None, None, None);

        let food = breakdown.iter().find(|s| s.name == "food").unwrap();
        assert_eq!(food.value, "80");

        let rent = breakdown.iter().find(|s| s.name == "rent").unwrap();
        assert_eq!(rent.value, "1000");
    }

    #[test]
    fn expense_breakdown_converts_currencies() {
        let input = "P 2024-01-01 EUR $1.10\n\n\
                     2024-01-10 A\n    expenses:food  $50\n    assets:checking\n\n\
                     2024-01-15 B\n    expenses:food  40 EUR\n    assets:eur  -40 EUR\n";
        let journal = parse(input).unwrap();
        let txns = resolve_transactions(&journal).unwrap();
        let db = PriceDb::from_journal(&journal);
        let breakdown = expense_breakdown(&txns, &db, "$", None, None, None);
        let food = breakdown.iter().find(|s| s.name == "food").unwrap();
        // 50 + 40*1.10 = 94, not 90 (raw cross-commodity sum).
        assert_eq!(food.value, "94");
    }

    #[test]
    fn expense_breakdown_drilldown() {
        let txns = resolve(
            "2024-01-10 A\n    expenses:food:groceries  $40\n    assets:checking\n\n\
             2024-01-15 B\n    expenses:food:dining  $30\n    assets:checking\n\n\
             2024-01-20 C\n    expenses:rent  $1000\n    assets:checking\n",
        );
        let breakdown =
            expense_breakdown(&txns, &no_prices(), "$", None, None, Some("expenses:food"));

        assert_eq!(breakdown.len(), 2);
        let groceries = breakdown.iter().find(|s| s.name == "groceries").unwrap();
        assert_eq!(groceries.value, "40");
        let dining = breakdown.iter().find(|s| s.name == "dining").unwrap();
        assert_eq!(dining.value, "30");
    }

    #[test]
    fn income_expense_series_basic() {
        let txns = resolve(
            "2024-01-15 Pay\n    assets:checking  $3000\n    income:salary\n\n\
             2024-01-20 Grocery\n    expenses:food  $50\n    assets:checking\n\n\
             2024-02-15 Pay\n    assets:checking  $3000\n    income:salary\n",
        );
        // A year-long range buckets by month.
        let series = income_expense_series(
            &txns,
            &classifier(),
            &no_prices(),
            "$",
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
            Some(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()),
            None,
        );

        assert!(series.len() >= 2);
        assert_eq!(series[0].period, "2024-01");
        assert_eq!(series[0].income, "3000");
        assert_eq!(series[0].expenses, "-50");
    }

    #[test]
    fn income_expense_series_breaks_down_short_ranges() {
        // Bucketing a single month by month gives one point and draws nothing;
        // a short range steps daily instead.
        let txns = resolve(
            "2024-01-05 Pay\n    assets:checking  $3000\n    income:salary\n\n\
             2024-01-20 Grocery\n    expenses:food  $50\n    assets:checking\n",
        );
        let series = income_expense_series(
            &txns,
            &classifier(),
            &no_prices(),
            "$",
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
            Some(NaiveDate::from_ymd_opt(2024, 1, 31).unwrap()),
            None,
        );

        assert_eq!(series.len(), 31, "expected one point per day");
        assert_eq!(series[4].period, "01-05");
        assert_eq!(series[4].income, "3000");
        assert_eq!(series[19].expenses, "-50");
    }

    #[test]
    fn series_step_shrinks_with_the_range() {
        let d = |m, day| NaiveDate::from_ymd_opt(2024, m, day).unwrap();
        assert_eq!(series_step(d(1, 1), d(1, 31)), SeriesStep::Day);
        assert_eq!(series_step(d(1, 1), d(3, 31)), SeriesStep::Week);
        assert_eq!(series_step(d(1, 1), d(12, 31)), SeriesStep::Month);
    }

    #[test]
    fn income_expense_series_respects_from_date() {
        let txns = resolve(
            "2024-01-05 Early\n    expenses:food  $100\n    assets:checking\n\n\
             2024-01-20 Late\n    expenses:food  $50\n    assets:checking\n",
        );
        let series = income_expense_series(
            &txns,
            &classifier(),
            &no_prices(),
            "$",
            Some(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()),
            None,
            None,
        );
        // Only the Jan 20 transaction is in range, whatever the bucket size.
        let total: rust_decimal::Decimal = series
            .iter()
            .map(|p| rust_decimal::Decimal::from_str_exact(&p.expenses).unwrap())
            .sum();
        assert_eq!(total, dec!(-50));
    }

    #[test]
    fn income_expense_refund_month_keeps_sign() {
        let txns = resolve(
            "2024-01-15 Refund\n    expenses:food  $-80\n    assets:checking\n",
        );
        let series = income_expense_series(&txns, &classifier(), &no_prices(), "$", None, None, None);
        // Net-refund month: expenses show positive 80, not -80.
        assert_eq!(series[0].expenses, "80");
    }

    #[test]
    fn net_worth_series_basic() {
        let txns = resolve(
            "2024-01-01 Opening\n    assets:checking  $1000\n    equity:opening\n\n\
             2024-02-01 Spend\n    expenses:food  $50\n    assets:checking\n",
        );
        let series =
            net_worth_series(&txns, &classifier(), &no_prices(), "$", None, None, None);

        // First and last rather than adjacent points, so the assertion holds
        // whatever bucket size the range selects.
        assert!(series.len() >= 2);
        assert_eq!(series.first().unwrap().value, "1000");
        assert_eq!(series.last().unwrap().value, "950");
    }

    #[test]
    fn net_worth_series_values_holdings() {
        let input = "P 2024-01-31 AAPL $150.00\nP 2024-02-28 AAPL $160.00\n\n\
                     2024-01-10 Buy\n    assets:stock  10 AAPL @ $140\n    assets:cash  $-1400\n\n\
                     2024-01-15 Fund\n    assets:cash  $2000\n    income:job\n";
        let journal = parse(input).unwrap();
        let txns = resolve_transactions(&journal).unwrap();
        let db = PriceDb::from_journal(&journal);
        let series = net_worth_series(
            &txns,
            &classifier(),
            &db,
            "$",
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
            Some(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()),
            None,
        );

        // End of Jan: cash 600 + 10 AAPL @150 = 2100 (not 610 raw-summed).
        assert_eq!(series[0].value, "2100");
    }

    #[test]
    fn net_worth_never_raw_sums_commodities() {
        // No prices at all: the foreign holding is excluded, not added raw.
        let txns = resolve(
            "2024-01-10 T\n    assets:cash  $600\n    income:job  $-600\n\n\
             2024-01-11 T2\n    assets:stock  10 XYZ\n    equity:conversion  -10 XYZ\n",
        );
        let series =
            net_worth_series(&txns, &classifier(), &no_prices(), "$", None, None, None);
        assert_eq!(series[0].value, "600");
    }

    #[test]
    fn unconvertible_reported() {
        let txns = resolve(
            "2024-01-10 T\n    assets:stock  10 XYZ\n    equity:conversion  -10 XYZ\n",
        );
        let list = unconvertible_commodities(
            &txns,
            "$",
            &no_prices(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
        );
        assert_eq!(list, vec!["XYZ".to_string()]);
    }

    #[test]
    fn end_of_month_works() {
        assert_eq!(
            end_of_month(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()),
            NaiveDate::from_ymd_opt(2024, 1, 31).unwrap()
        );
        assert_eq!(
            end_of_month(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()),
            NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()
        );
        assert_eq!(
            end_of_month(NaiveDate::from_ymd_opt(2024, 12, 5).unwrap()),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()
        );
    }

    #[test]
    fn audit_cost_transaction_balances() {
        let txns = resolve(
            "2025-02-16 * Sell shares of ITOT\n\
             \x20   Assets:US:ETrade:ITOT    -19 ITOT {96.15 USD}\n\
             \x20   Assets:US:ETrade:Cash    1973.70 USD\n\
             \x20   Expenses:Financial:Commissions    8.95 USD\n\
             \x20   Income:US:ETrade:PnL\n",
        );
        assert_eq!(txns.len(), 1);
        let t = &txns[0];
        assert_eq!(t.postings[0].amount.get("ITOT"), dec!(-19));
        assert_eq!(t.postings[1].amount.get("USD"), dec!(1973.70));
        // PnL inferred from the cost-converted sum: -155.80 USD.
        assert_eq!(t.postings[3].amount.get("USD"), dec!(-155.80));
    }

    #[test]
    fn audit_example_hledger_asset_balances() {
        let text = std::fs::read_to_string("../../tests/fixtures/example.hledger").unwrap();
        let journal = hledger_parser::parse(&text).expect("parse failed");
        let journal_txns = crate::balance::resolve_transactions(&journal).expect("resolve failed");
        let report = balance_report(&journal_txns, Some("assets"), None, None);

        let find = |name: &str| report.iter().find(|r| r.account == name)
            .unwrap_or_else(|| panic!("Account {} not found in report", name));
        let has_amt = |row: &BalanceRow, commodity: &str, expected: &str| {
            let expected_dec = rust_decimal::Decimal::from_str_exact(expected).unwrap();
            row.amounts.iter().any(|a| {
                a.commodity == commodity
                    && rust_decimal::Decimal::from_str_exact(&a.quantity).unwrap() == expected_dec
            })
        };

        let checking = find("Assets:US:BofA:Checking");
        assert!(has_amt(checking, "USD", "1869.39"),
            "BofA Checking: expected 1869.39 USD, got {:?}", checking.amounts);

        let etrade_cash = find("Assets:US:ETrade:Cash");
        assert!(has_amt(etrade_cash, "USD", "5724.75"),
            "ETrade Cash: expected 5724.75 USD, got {:?}", etrade_cash.amounts);

        let gld = find("Assets:US:ETrade:GLD");
        assert!(has_amt(gld, "GLD", "45"), "GLD: got {:?}", gld.amounts);

        let itot = find("Assets:US:ETrade:ITOT");
        assert!(has_amt(itot, "ITOT", "62"), "ITOT: got {:?}", itot.amounts);

        let vht = find("Assets:US:ETrade:VHT");
        assert!(has_amt(vht, "VHT", "76"), "VHT: got {:?}", vht.amounts);

        let rgagx = find("Assets:US:Vanguard:RGAGX");
        assert!(has_amt(rgagx, "RGAGX", "284.123"), "RGAGX: got {:?}", rgagx.amounts);

        let vbmpx = find("Assets:US:Vanguard:VBMPX");
        assert!(has_amt(vbmpx, "VBMPX", "169.659"), "VBMPX: got {:?}", vbmpx.amounts);
    }

    #[test]
    fn audit_valued_balance_report() {
        let text = std::fs::read_to_string("../../tests/fixtures/example.hledger").unwrap();
        let journal = hledger_parser::parse(&text).expect("parse failed");
        let price_db = crate::price_db::PriceDb::from_journal(&journal);
        let journal_txns = crate::balance::resolve_transactions(&journal).expect("resolve failed");

        let report = balance_report_valued(&journal_txns, Some("assets"), None, None, "USD", &price_db);

        let find = |name: &str| report.iter().find(|r| r.account == name)
            .unwrap_or_else(|| panic!("Account {} not found", name));
        let get_usd = |row: &BalanceRow| -> f64 {
            row.amounts.iter()
                .find(|a| a.commodity == "USD")
                .map(|a| a.quantity.parse::<f64>().unwrap())
                .unwrap_or(0.0)
        };

        let gld = find("Assets:US:ETrade:GLD");
        assert!((get_usd(gld) - 2054.25).abs() < 1.0,
            "GLD valued: expected ~2054.25 USD, got {}", get_usd(gld));

        let itot = find("Assets:US:ETrade:ITOT");
        assert!((get_usd(itot) - 5476.46).abs() < 1.0,
            "ITOT valued: expected ~5476.46 USD, got {}", get_usd(itot));

        let checking = find("Assets:US:BofA:Checking");
        assert!((get_usd(checking) - 1869.39).abs() < 0.01,
            "Checking: expected 1869.39 USD, got {}", get_usd(checking));
    }

    #[test]
    fn audit_multicommodity_balance_report() {
        let txns = resolve(
            "2025-01-01 Buy stock\n\
             \x20   Assets:Brokerage:Stock    10 AAPL @ 150 USD\n\
             \x20   Assets:Brokerage:Cash    -1500 USD\n\n\
             2025-01-15 Deposit\n\
             \x20   Assets:Brokerage:Cash    5000 USD\n\
             \x20   Income:Salary\n",
        );

        let report = balance_report(&txns, Some("assets"), None, None);
        let stock = report.iter().find(|r| r.account == "Assets:Brokerage:Stock").unwrap();
        assert!(stock.amounts.iter().any(|a| a.commodity == "AAPL" && a.quantity == "10"));

        let cash = report.iter().find(|r| r.account == "Assets:Brokerage:Cash").unwrap();
        assert!(cash.amounts.iter().any(|a| a.commodity == "USD" && a.quantity == "3500"));
    }

    #[test]
    fn audit_income_statement_net() {
        let text = std::fs::read_to_string("../../tests/fixtures/example.hledger").unwrap();
        let journal = hledger_parser::parse(&text).expect("parse failed");
        let txns = crate::balance::resolve_transactions(&journal).expect("resolve failed");
        let c = AccountClassifier::from_journal(&journal);

        let from = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2025, 2, 28).unwrap();
        let is = income_statement(&txns, &c, &no_prices(), "", Some(from), Some(to));

        let net_usd = is.net.iter().find(|a| a.commodity == "USD");
        if let Some(n) = net_usd {
            let val: f64 = n.quantity.parse().unwrap();
            assert!((val - 3089.64).abs() < 0.01,
                "IS net USD: expected 3089.64, got {}", val);
        }
    }
}

/// Summary facts about a journal — the shape of `hledger stats`, plus the
/// account activity Fava's Statistics report shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalStatistics {
    pub transaction_count: usize,
    pub posting_count: usize,
    /// Accounts actually posted to, not the full tree including parents.
    pub account_count: usize,
    pub commodities: Vec<String>,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
    pub days_covered: i64,
    /// Mean transactions per month over the covered span.
    pub per_month: String,
    /// Busiest accounts by posting count, most first.
    pub busiest_accounts: Vec<AccountActivity>,
    /// Postings per month across the whole journal, oldest first.
    pub activity: Vec<ActivityPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountActivity {
    pub account: String,
    pub postings: usize,
    /// Date of the most recent posting to this account.
    pub last_seen: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPoint {
    pub period: String,
    pub postings: usize,
}

pub fn journal_statistics(transactions: &[ResolvedTransaction]) -> JournalStatistics {
    let mut posting_count = 0usize;
    let mut accounts: BTreeMap<String, (usize, NaiveDate)> = BTreeMap::new();
    let mut commodities: BTreeSet<String> = BTreeSet::new();
    let mut per_period: BTreeMap<String, usize> = BTreeMap::new();
    let mut first: Option<NaiveDate> = None;
    let mut last: Option<NaiveDate> = None;

    for txn in transactions {
        first = Some(first.map_or(txn.date, |d: NaiveDate| d.min(txn.date)));
        last = Some(last.map_or(txn.date, |d: NaiveDate| d.max(txn.date)));
        for posting in &txn.postings {
            posting_count += 1;
            let entry = accounts
                .entry(posting.account.full.clone())
                .or_insert((0, posting.date));
            entry.0 += 1;
            entry.1 = entry.1.max(posting.date);
            for commodity in posting.amount.amounts.keys() {
                if !commodity.is_empty() {
                    commodities.insert(commodity.clone());
                }
            }
            *per_period
                .entry(posting.date.format("%Y-%m").to_string())
                .or_insert(0) += 1;
        }
    }

    let days_covered = match (first, last) {
        (Some(f), Some(l)) => (l - f).num_days() + 1,
        _ => 0,
    };
    // Months rather than days: a per-day rate reads as zero for most journals.
    let months = (days_covered as f64 / 30.44).max(1.0);
    let per_month = format!("{:.1}", transactions.len() as f64 / months);

    let mut busiest: Vec<AccountActivity> = accounts
        .into_iter()
        .map(|(account, (postings, last_seen))| AccountActivity {
            account,
            postings,
            last_seen: last_seen.format("%Y-%m-%d").to_string(),
        })
        .collect();
    busiest.sort_by(|a, b| b.postings.cmp(&a.postings).then(a.account.cmp(&b.account)));
    let account_count = busiest.len();
    busiest.truncate(15);

    JournalStatistics {
        transaction_count: transactions.len(),
        posting_count,
        account_count,
        commodities: commodities.into_iter().collect(),
        first_date: first.map(|d| d.format("%Y-%m-%d").to_string()),
        last_date: last.map(|d| d.format("%Y-%m-%d").to_string()),
        days_covered,
        per_month,
        busiest_accounts: busiest,
        activity: per_period
            .into_iter()
            .map(|(period, postings)| ActivityPoint { period, postings })
            .collect(),
    }
}

/// How to value a balance, mirroring hledger's balance calculation modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValuationMode {
    /// The units held, unconverted (hledger's default).
    Units,
    /// What was paid: each posting's cost where one was written, else the
    /// posting itself (`-B` / `--cost`).
    Cost,
    /// Worth at market prices on the valuation date (`-V` / `-X`).
    Market,
    /// Unrealised capital gain: market value minus cost basis (`--gain`).
    Gain,
}

pub fn parse_valuation_mode(name: &str) -> Option<ValuationMode> {
    match name {
        "units" => Some(ValuationMode::Units),
        "cost" => Some(ValuationMode::Cost),
        "market" | "value" => Some(ValuationMode::Market),
        "gain" => Some(ValuationMode::Gain),
        _ => None,
    }
}

/// What a posting cost: its cost amount when written with `@`/`@@`, else the
/// posting's own amount. A cash posting has no cost of its own — the money is
/// the cost.
pub fn posting_cost(posting: &crate::balance::ResolvedPosting) -> MixedAmount {
    posting.cost.clone().unwrap_or_else(|| posting.amount.clone())
}

/// A balance report in one of hledger's valuation modes.
///
/// `Gain` is the interesting one: it answers "how much of this balance is
/// profit I haven't taken yet", which needs both sides — what the holding is
/// worth now and what it cost — and is meaningless in the units the holding
/// is denominated in.
pub fn balance_report_mode(
    transactions: &[ResolvedTransaction],
    account_filter: Option<&str>,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    target_commodity: &str,
    price_db: &PriceDb,
    mode: ValuationMode,
) -> Vec<BalanceRow> {
    let valuation_date = date_to.unwrap_or_else(|| chrono::Local::now().date_naive());

    let mut units: BTreeMap<String, MixedAmount> = BTreeMap::new();
    let mut costs: BTreeMap<String, MixedAmount> = BTreeMap::new();

    for txn in transactions {
        for posting in &txn.postings {
            if let Some(filter) = account_filter {
                if !account_matches_prefix(&posting.account.full, filter) {
                    continue;
                }
            }
            if !date_in_range(posting.date, date_from, date_to) {
                continue;
            }
            units
                .entry(posting.account.full.clone())
                .or_insert_with(MixedAmount::zero)
                .add_mixed(&posting.amount);
            costs
                .entry(posting.account.full.clone())
                .or_insert_with(MixedAmount::zero)
                .add_mixed(&posting_cost(posting));
        }
    }

    let valued: BTreeMap<String, MixedAmount> = units
        .iter()
        .map(|(account, held)| {
            let amount = match mode {
                ValuationMode::Units => held.clone(),
                ValuationMode::Cost => costs.get(account).cloned().unwrap_or_default(),
                ValuationMode::Market => {
                    convert_mixed(held, target_commodity, price_db, valuation_date)
                }
                ValuationMode::Gain => {
                    let market =
                        convert_mixed(held, target_commodity, price_db, valuation_date);
                    let cost = costs.get(account).cloned().unwrap_or_default();
                    let mut gain = market;
                    gain.subtract(&cost);
                    gain
                }
            };
            (account.clone(), amount)
        })
        .collect();

    rows_with_parents(valued)
}
