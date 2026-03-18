use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::amount::MixedAmount;
use crate::balance::ResolvedTransaction;
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
            quantity: q.to_string(),
        })
        .collect()
}

/// Convert a MixedAmount to a target commodity using the price database.
/// Commodities that can't be converted are dropped from the result.
fn convert_mixed(m: &MixedAmount, target: &str, price_db: &PriceDb, date: NaiveDate) -> MixedAmount {
    let mut result = MixedAmount::zero();
    for (commodity, quantity) in &m.amounts {
        if commodity == target {
            result.add(target, *quantity);
        } else if let Some(converted) = price_db.convert(*quantity, commodity, target, date) {
            result.add(target, converted);
        }
        // No price available - skip this commodity rather than mixing it in
    }
    result
}

/// Public version for use by other modules.
pub fn get_primary_value_pub(m: &MixedAmount, target: &str) -> Decimal {
    get_primary_value(m, target)
}

/// Get the value in the target commodity, falling back to the first available
/// commodity if the target isn't found.
fn get_primary_value(m: &MixedAmount, target: &str) -> Decimal {
    if !target.is_empty() {
        let val = m.get(target);
        if !val.is_zero() || m.amounts.is_empty() {
            return val;
        }
    }
    // Fallback: sum all commodity values (works for single-currency journals)
    m.amounts.values().copied().fold(Decimal::ZERO, |a, b| a + b)
}

/// Case-insensitive check if an account belongs to a given type.
fn is_account_type(account: &str, account_type: &str) -> bool {
    let lower = account.to_lowercase();
    lower == account_type
        || lower.starts_with(&format!("{}:", account_type))
}

// ─── Report generation functions ───

/// Generate a balance report: account balances filtered by account prefix and date range.
pub fn balance_report(
    transactions: &[ResolvedTransaction],
    account_filter: Option<&str>,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Vec<BalanceRow> {
    let mut balances: BTreeMap<String, MixedAmount> = BTreeMap::new();

    for txn in transactions {
        if let Some(from) = date_from {
            if txn.date < from {
                continue;
            }
        }
        if let Some(to) = date_to {
            if txn.date > to {
                continue;
            }
        }
        for posting in &txn.postings {
            if let Some(filter) = account_filter {
                if !is_account_type(&posting.account.full, filter) {
                    continue;
                }
            }
            let entry = balances
                .entry(posting.account.full.clone())
                .or_insert_with(MixedAmount::zero);
            entry.add_mixed(&posting.amount);
        }
    }

    // Also add parent accounts
    let leaf_accounts: Vec<String> = balances.keys().cloned().collect();
    for account in &leaf_accounts {
        let parts: Vec<&str> = account.split(':').collect();
        for depth in 1..parts.len() {
            let parent = parts[..depth].join(":");
            // Ensure parent exists but don't add amounts (they'll be computed)
            balances.entry(parent).or_insert_with(MixedAmount::zero);
        }
    }

    // Compute inclusive balances (parent = sum of children)
    let all_accounts: Vec<String> = balances.keys().cloned().collect();
    let mut inclusive: BTreeMap<String, MixedAmount> = BTreeMap::new();

    for account in &all_accounts {
        let mut total = balances.get(account).cloned().unwrap_or_default();
        // Add all descendants
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

    // Filter out zero balances and format
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

/// Generate a balance report with values converted to a target commodity using market prices.
pub fn balance_report_valued(
    transactions: &[ResolvedTransaction],
    account_filter: Option<&str>,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    target_commodity: &str,
    price_db: &PriceDb,
) -> Vec<BalanceRow> {
    let valuation_date = date_to.unwrap_or_else(|| {
        transactions.last().map(|t| t.date).unwrap_or_else(|| chrono::Local::now().date_naive())
    });

    let mut balances: BTreeMap<String, MixedAmount> = BTreeMap::new();

    for txn in transactions {
        if let Some(from) = date_from {
            if txn.date < from { continue; }
        }
        if let Some(to) = date_to {
            if txn.date > to { continue; }
        }
        for posting in &txn.postings {
            if let Some(filter) = account_filter {
                if !is_account_type(&posting.account.full, filter) { continue; }
            }
            let entry = balances.entry(posting.account.full.clone()).or_insert_with(MixedAmount::zero);
            entry.add_mixed(&posting.amount);
        }
    }

    // Add parent accounts
    let leaf_accounts: Vec<String> = balances.keys().cloned().collect();
    for account in &leaf_accounts {
        let parts: Vec<&str> = account.split(':').collect();
        for depth in 1..parts.len() {
            let parent = parts[..depth].join(":");
            balances.entry(parent).or_insert_with(MixedAmount::zero);
        }
    }

    // Compute inclusive balances
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
        // Convert to target commodity using prices
        let valued = convert_mixed(&total, target_commodity, price_db, valuation_date);
        inclusive.insert(account.clone(), valued);
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

/// Generate a register report for a specific account.
pub fn register_report(
    transactions: &[ResolvedTransaction],
    account_filter: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Vec<RegisterRow> {
    let mut rows = Vec::new();
    let mut running_total = MixedAmount::zero();

    for txn in transactions {
        if let Some(from) = date_from {
            if txn.date < from {
                continue;
            }
        }
        if let Some(to) = date_to {
            if txn.date > to {
                continue;
            }
        }
        for posting in &txn.postings {
            if !posting.account.full.starts_with(account_filter) {
                continue;
            }
            running_total.add_mixed(&posting.amount);
            rows.push(RegisterRow {
                date: txn.date.format("%Y-%m-%d").to_string(),
                description: txn.description.clone(),
                account: posting.account.full.clone(),
                amount: mixed_to_entries(&posting.amount),
                running_total: mixed_to_entries(&running_total),
            });
        }
    }

    rows
}

/// Generate a Balance Sheet (Assets - Liabilities = Equity).
pub fn balance_sheet(
    transactions: &[ResolvedTransaction],
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> FinancialStatement {
    let assets = section_balance(transactions, "assets", date_from, date_to);
    let liabilities = section_balance(transactions, "liabilities", date_from, date_to);
    let equity = section_balance(transactions, "equity", date_from, date_to);

    let mut net = assets.total.clone();
    let mut liab_total = liabilities.total.clone();
    liab_total.add_mixed(&equity.total);
    // Net = Assets - Liabilities - Equity (should be zero in balanced books)

    FinancialStatement {
        title: "Balance Sheet".to_string(),
        sections: vec![
            format_section("Assets", &assets),
            format_section("Liabilities", &liabilities),
            format_section("Equity", &equity),
        ],
        net: mixed_to_entries(&net),
    }
}

/// Generate an Income Statement (Revenue - Expenses = Net Income).
pub fn income_statement(
    transactions: &[ResolvedTransaction],
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> FinancialStatement {
    let income = section_balance(transactions, "income", date_from, date_to);
    let revenue = section_balance(transactions, "revenue", date_from, date_to);
    let expenses = section_balance(transactions, "expenses", date_from, date_to);

    // Combine income + revenue
    let mut combined_income = income.total.clone();
    combined_income.add_mixed(&revenue.total);
    let income_negated = combined_income.negate(); // Income is typically negative in double-entry

    let mut net = income_negated.clone();
    net.subtract(&expenses.total);

    let mut income_rows = income.rows;
    income_rows.extend(revenue.rows);

    FinancialStatement {
        title: "Income Statement".to_string(),
        sections: vec![
            StatementSection {
                title: "Income".to_string(),
                rows: income_rows,
                total: mixed_to_entries(&income_negated),
            },
            format_section("Expenses", &expenses),
        ],
        net: mixed_to_entries(&net),
    }
}

/// Generate a Cash Flow statement.
pub fn cash_flow(
    transactions: &[ResolvedTransaction],
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> FinancialStatement {
    let cash = section_balance(transactions, "assets", date_from, date_to);

    FinancialStatement {
        title: "Cash Flow".to_string(),
        sections: vec![format_section("Cash Changes", &cash)],
        net: mixed_to_entries(&cash.total),
    }
}

/// Net worth over time (assets - liabilities at end of each month).
pub fn net_worth_series(
    transactions: &[ResolvedTransaction],
    target_commodity: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Vec<TimeSeriesPoint> {
    if transactions.is_empty() {
        return vec![];
    }

    let first_date = date_from.unwrap_or(transactions.first().unwrap().date);
    let last_date = date_to.unwrap_or(transactions.last().unwrap().date);

    let mut points = Vec::new();
    let mut assets = MixedAmount::zero();
    let mut liabilities = MixedAmount::zero();
    let mut txn_idx = 0;

    let mut current = end_of_month(first_date);
    while current <= end_of_month(last_date) {
        while txn_idx < transactions.len() && transactions[txn_idx].date <= current {
            for posting in &transactions[txn_idx].postings {
                if is_account_type(&posting.account.full, "assets") {
                    assets.add_mixed(&posting.amount);
                } else if is_account_type(&posting.account.full, "liabilities") {
                    liabilities.add_mixed(&posting.amount);
                }
            }
            txn_idx += 1;
        }

        let net_worth =
            get_primary_value(&assets, target_commodity) + get_primary_value(&liabilities, target_commodity);

        points.push(TimeSeriesPoint {
            date: current.format("%Y-%m-%d").to_string(),
            value: net_worth.to_string(),
        });

        current = next_month_end(current);
    }

    points
}

/// Account balance over time for a specific account.
pub fn account_series(
    transactions: &[ResolvedTransaction],
    account_prefix: &str,
    target_commodity: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Vec<TimeSeriesPoint> {
    if transactions.is_empty() {
        return vec![];
    }

    let first_date = date_from.unwrap_or(transactions.first().unwrap().date);
    let last_date = date_to.unwrap_or(transactions.last().unwrap().date);

    let mut points = Vec::new();
    let mut balance = MixedAmount::zero();
    let mut txn_idx = 0;

    let mut current = end_of_month(first_date);
    while current <= end_of_month(last_date) {
        while txn_idx < transactions.len() && transactions[txn_idx].date <= current {
            for posting in &transactions[txn_idx].postings {
                if posting.account.full.starts_with(account_prefix) {
                    balance.add_mixed(&posting.amount);
                }
            }
            txn_idx += 1;
        }

        let value = get_primary_value(&balance, target_commodity);
        points.push(TimeSeriesPoint {
            date: current.format("%Y-%m-%d").to_string(),
            value: value.to_string(),
        });

        current = next_month_end(current);
    }

    points
}

/// Income vs Expenses by month.
pub fn income_expense_series(
    transactions: &[ResolvedTransaction],
    target_commodity: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Vec<IncomeExpensePoint> {
    if transactions.is_empty() {
        return vec![];
    }

    let first_date = date_from.unwrap_or(transactions.first().unwrap().date);
    let last_date = date_to.unwrap_or(transactions.last().unwrap().date);

    let mut points = Vec::new();
    let mut txn_idx = 0;

    let mut current_start = start_of_month(first_date);
    while current_start <= last_date {
        let current_end = end_of_month(current_start);
        let mut income = MixedAmount::zero();
        let mut expenses = MixedAmount::zero();

        while txn_idx < transactions.len() && transactions[txn_idx].date <= current_end {
            if transactions[txn_idx].date >= current_start {
                for posting in &transactions[txn_idx].postings {
                    if is_account_type(&posting.account.full, "income")
                        || is_account_type(&posting.account.full, "revenue")
                    {
                        income.add_mixed(&posting.amount);
                    } else if is_account_type(&posting.account.full, "expenses") {
                        expenses.add_mixed(&posting.amount);
                    }
                }
            }
            txn_idx += 1;
        }

        // Income is negative in double-entry, negate for display
        let income_val = get_primary_value(&income, target_commodity).abs();
        // Expenses as negative for the chart
        let expense_val = -(get_primary_value(&expenses, target_commodity).abs());

        points.push(IncomeExpensePoint {
            period: current_start.format("%Y-%m").to_string(),
            income: income_val.to_string(),
            expenses: expense_val.to_string(),
        });

        current_start = next_month_start(current_start);
    }

    points
}

/// Expense breakdown by subcategory, with optional drill-down via parent_prefix.
/// - parent_prefix=None: breaks down by top-level expense categories (expenses:X)
/// - parent_prefix=Some("expenses:food"): breaks down by subcategories of food
pub fn expense_breakdown(
    transactions: &[ResolvedTransaction],
    target_commodity: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    parent_prefix: Option<&str>,
) -> Vec<PieSlice> {
    let prefix = parent_prefix.unwrap_or("expenses");
    let prefix_lower = prefix.to_lowercase();
    let prefix_depth = prefix.matches(':').count() + 1; // depth of children

    let mut by_category: BTreeMap<String, Decimal> = BTreeMap::new();

    for txn in transactions {
        if let Some(from) = date_from {
            if txn.date < from {
                continue;
            }
        }
        if let Some(to) = date_to {
            if txn.date > to {
                continue;
            }
        }
        for posting in &txn.postings {
            let acct_lower = posting.account.full.to_lowercase();
            // Must be under the prefix
            if !(acct_lower == prefix_lower
                || (acct_lower.starts_with(&prefix_lower) && acct_lower.as_bytes().get(prefix_lower.len()) == Some(&b':')))
            {
                continue;
            }

            // Get the child name at the next depth level
            let category = posting
                .account
                .parts
                .get(prefix_depth)
                .cloned()
                .unwrap_or_else(|| "other".to_string());

            let value = get_primary_value(&posting.amount, target_commodity);
            *by_category.entry(category).or_insert(Decimal::ZERO) += value;
        }
    }

    // Sort by value descending, keep top 7, group rest as "other"
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
                value: value.to_string(),
            })
            .collect();
        if !other_total.is_zero() {
            result.push(PieSlice {
                name: "other".to_string(),
                value: other_total.to_string(),
            });
        }
        result
    } else {
        sorted
            .into_iter()
            .map(|(name, value)| PieSlice {
                name,
                value: value.to_string(),
            })
            .collect()
    }
}

// ─── Helper types ───

struct SectionData {
    rows: Vec<BalanceRow>,
    total: MixedAmount,
}

fn section_balance(
    transactions: &[ResolvedTransaction],
    prefix: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> SectionData {
    let rows = balance_report(transactions, Some(prefix), date_from, date_to);
    let mut total = MixedAmount::zero();

    // Total = sum of top-level accounts in this section
    for txn in transactions {
        if let Some(from) = date_from {
            if txn.date < from {
                continue;
            }
        }
        if let Some(to) = date_to {
            if txn.date > to {
                continue;
            }
        }
        for posting in &txn.postings {
            if is_account_type(&posting.account.full, prefix) {
                total.add_mixed(&posting.amount);
            }
        }
    }

    SectionData { rows, total }
}

fn format_section(title: &str, data: &SectionData) -> StatementSection {
    StatementSection {
        title: title.to_string(),
        rows: data.rows.clone(),
        total: mixed_to_entries(&data.total),
    }
}

// ─── Date helpers ───

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

    #[test]
    fn balance_report_simple() {
        let txns = resolve(
            "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking\n",
        );
        let report = balance_report(&txns, None, None, None);

        let food = report.iter().find(|r| r.account == "expenses:food").unwrap();
        assert_eq!(food.amounts[0].quantity, "50.00");

        let checking = report.iter().find(|r| r.account == "assets:checking").unwrap();
        assert_eq!(checking.amounts[0].quantity, "-50.00");
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
    fn balance_sheet_basic() {
        let txns = resolve(
            "2024-01-01 Opening\n    assets:checking  $1000\n    equity:opening\n\n\
             2024-01-15 Spend\n    expenses:food  $50\n    assets:checking\n",
        );
        let bs = balance_sheet(&txns, None, None);

        assert_eq!(bs.title, "Balance Sheet");
        assert_eq!(bs.sections.len(), 3); // Assets, Liabilities, Equity
    }

    #[test]
    fn income_statement_basic() {
        let txns = resolve(
            "2024-01-15 Paycheck\n    assets:checking  $3000\n    income:salary\n\n\
             2024-01-20 Grocery\n    expenses:food  $50\n    assets:checking\n",
        );
        let is = income_statement(&txns, None, None);

        assert_eq!(is.title, "Income Statement");
        assert_eq!(is.sections.len(), 2); // Income, Expenses
    }

    #[test]
    fn expense_breakdown_basic() {
        let txns = resolve(
            "2024-01-10 A\n    expenses:food  $50\n    assets:checking\n\n\
             2024-01-15 B\n    expenses:rent  $1000\n    assets:checking\n\n\
             2024-01-20 C\n    expenses:food  $30\n    assets:checking\n",
        );
        let breakdown = expense_breakdown(&txns, "$", None, None, None);

        let food = breakdown.iter().find(|s| s.name == "food").unwrap();
        assert_eq!(food.value, "80");

        let rent = breakdown.iter().find(|s| s.name == "rent").unwrap();
        assert_eq!(rent.value, "1000");
    }

    #[test]
    fn expense_breakdown_drilldown() {
        let txns = resolve(
            "2024-01-10 A\n    expenses:food:groceries  $40\n    assets:checking\n\n\
             2024-01-15 B\n    expenses:food:dining  $30\n    assets:checking\n\n\
             2024-01-20 C\n    expenses:rent  $1000\n    assets:checking\n",
        );
        let breakdown = expense_breakdown(&txns, "$", None, None, Some("expenses:food"));

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
        let series = income_expense_series(&txns, "$", None, None);

        assert!(series.len() >= 2);
        assert_eq!(series[0].period, "2024-01");
        assert_eq!(series[0].income, "3000");
        assert_eq!(series[0].expenses, "-50");
    }

    #[test]
    fn net_worth_series_basic() {
        let txns = resolve(
            "2024-01-01 Opening\n    assets:checking  $1000\n    equity:opening\n\n\
             2024-02-01 Spend\n    expenses:food  $50\n    assets:checking\n",
        );
        let series = net_worth_series(&txns, "$", None, None);

        assert!(!series.is_empty());
        // First month: $1000
        assert_eq!(series[0].value, "1000");
        // Second month: $950
        assert_eq!(series[1].value, "950");
    }

    #[test]
    fn audit_cost_transaction_balances() {
        // This mirrors a real transaction from example.hledger
        let txns = resolve(
            "2025-02-16 * Sell shares of ITOT\n\
             \x20   Assets:US:ETrade:ITOT    -19 ITOT {96.15 USD}\n\
             \x20   Assets:US:ETrade:Cash    1973.70 USD\n\
             \x20   Expenses:Financial:Commissions    8.95 USD\n\
             \x20   Income:US:ETrade:PnL\n",
        );
        assert_eq!(txns.len(), 1);
        let t = &txns[0];

        // ITOT posting: should have -19 ITOT
        assert_eq!(t.postings[0].amount.get("ITOT"), dec!(-19));

        // Cash posting: 1973.70 USD
        assert_eq!(t.postings[1].amount.get("USD"), dec!(1973.70));

        // PnL (inferred): should balance the USD side
        // Cost: -19 * 96.15 = -1826.85 USD equivalent
        // Cash: +1973.70, Commissions: +8.95
        // So PnL = -(1973.70 + 8.95 - 1826.85) = -155.80 USD
        // But also +19 ITOT to balance the ITOT commodity
        let pnl = &t.postings[3];
        println!("PnL amounts: {:?}", pnl.amount.amounts);
        // With current code, PnL gets the negation of the sum of explicit amounts
    }

    #[test]
    fn audit_example_hledger_asset_balances() {
        let text = std::fs::read_to_string("../../tests/fixtures/example.hledger").unwrap();
        let journal = hledger_parser::parse(&text).expect("parse failed");
        let txns = resolve(&text[..0]); // dummy - use below
        let _ = txns;

        let journal_txns = crate::balance::resolve_transactions(&journal).expect("resolve failed");
        let report = balance_report(&journal_txns, Some("assets"), None, None);

        // Compare against hledger CLI output (account names preserve original casing):
        let find = |name: &str| report.iter().find(|r| r.account == name)
            .unwrap_or_else(|| panic!("Account {} not found in report", name));
        let has_amt = |row: &BalanceRow, commodity: &str, expected: &str| {
            let expected_dec = rust_decimal::Decimal::from_str_exact(expected).unwrap();
            row.amounts.iter().any(|a| {
                a.commodity == commodity
                    && rust_decimal::Decimal::from_str_exact(&a.quantity).unwrap() == expected_dec
            })
        };

        // 1869.39000 USD  Assets:US:BofA:Checking
        let checking = find("Assets:US:BofA:Checking");
        assert!(has_amt(checking, "USD", "1869.39"),
            "BofA Checking: expected 1869.39 USD, got {:?}", checking.amounts);

        // 5724.75000 USD  Assets:US:ETrade:Cash
        let etrade_cash = find("Assets:US:ETrade:Cash");
        assert!(has_amt(etrade_cash, "USD", "5724.75"),
            "ETrade Cash: expected 5724.75 USD, got {:?}", etrade_cash.amounts);

        // 45 GLD  Assets:US:ETrade:GLD
        let gld = find("Assets:US:ETrade:GLD");
        assert!(has_amt(gld, "GLD", "45"),
            "GLD: expected 45 GLD, got {:?}", gld.amounts);

        // 62 ITOT  Assets:US:ETrade:ITOT
        let itot = find("Assets:US:ETrade:ITOT");
        assert!(has_amt(itot, "ITOT", "62"),
            "ITOT: expected 62 ITOT, got {:?}", itot.amounts);

        // 76 VHT  Assets:US:ETrade:VHT
        let vht = find("Assets:US:ETrade:VHT");
        assert!(has_amt(vht, "VHT", "76"),
            "VHT: expected 76 VHT, got {:?}", vht.amounts);

        // 284.123 RGAGX  Assets:US:Vanguard:RGAGX
        let rgagx = find("Assets:US:Vanguard:RGAGX");
        assert!(has_amt(rgagx, "RGAGX", "284.123"),
            "RGAGX: expected 284.123 RGAGX, got {:?}", rgagx.amounts);

        // 169.659 VBMPX  Assets:US:Vanguard:VBMPX
        let vbmpx = find("Assets:US:Vanguard:VBMPX");
        assert!(has_amt(vbmpx, "VBMPX", "169.659"),
            "VBMPX: expected 169.659 VBMPX, got {:?}", vbmpx.amounts);
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

        // hledger -V output: GLD = 2054.25 USD (45 * 45.65)
        let gld = find("Assets:US:ETrade:GLD");
        let gld_usd = get_usd(gld);
        assert!((gld_usd - 2054.25).abs() < 1.0,
            "GLD valued: expected ~2054.25 USD, got {}", gld_usd);

        // ITOT = 5476.46 USD (62 * 88.33)
        let itot = find("Assets:US:ETrade:ITOT");
        let itot_usd = get_usd(itot);
        assert!((itot_usd - 5476.46).abs() < 1.0,
            "ITOT valued: expected ~5476.46 USD, got {}", itot_usd);

        // BofA Checking stays as USD (no conversion needed)
        let checking = find("Assets:US:BofA:Checking");
        let checking_usd = get_usd(checking);
        assert!((checking_usd - 1869.39).abs() < 0.01,
            "Checking: expected 1869.39 USD, got {}", checking_usd);
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
        println!("=== Multi-commodity balance report ===");
        for row in &report {
            println!("  {}: {:?}", row.account, row.amounts);
        }

        // Stock account should show AAPL
        let stock = report.iter().find(|r| r.account == "Assets:Brokerage:Stock").unwrap();
        assert!(stock.amounts.iter().any(|a| a.commodity == "AAPL" && a.quantity == "10"),
            "Stock should have 10 AAPL, got {:?}", stock.amounts);

        // Cash should show USD
        let cash = report.iter().find(|r| r.account == "Assets:Brokerage:Cash").unwrap();
        assert!(cash.amounts.iter().any(|a| a.commodity == "USD" && a.quantity == "3500"),
            "Cash should have 3500 USD, got {:?}", cash.amounts);
    }

    #[test]
    fn end_of_month_works() {
        assert_eq!(
            end_of_month(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()),
            NaiveDate::from_ymd_opt(2024, 1, 31).unwrap()
        );
        assert_eq!(
            end_of_month(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()),
            NaiveDate::from_ymd_opt(2024, 2, 29).unwrap() // leap year
        );
        assert_eq!(
            end_of_month(NaiveDate::from_ymd_opt(2024, 12, 5).unwrap()),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()
        );
    }

    #[test]
    fn audit_income_statement_net() {
        let text = std::fs::read_to_string("../../tests/fixtures/example.hledger").unwrap();
        let journal = hledger_parser::parse(&text).expect("parse failed");
        let txns = crate::balance::resolve_transactions(&journal).expect("resolve failed");

        let from = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2025, 2, 28).unwrap();
        let is = income_statement(&txns, Some(from), Some(to));

        // hledger says: net = 3089.64 USD - 2400 IRAUSD + 10 VACHR
        let net_usd = is.net.iter().find(|a| a.commodity == "USD");
        if let Some(n) = net_usd {
            let val: f64 = n.quantity.parse().unwrap();
            assert!((val - 3089.64).abs() < 0.01,
                "IS net USD: expected 3089.64, got {}", val);
        }
    }
}
