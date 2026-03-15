use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::amount::MixedAmount;
use crate::balance::ResolvedTransaction;
use crate::reports::get_primary_value_pub;

use hledger_parser::ast::{Journal, JournalItem};

/// Budget period types mapped from hledger periodic transaction syntax.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetPeriod {
    Weekly,
    Monthly,
    Quarterly,
    Yearly,
}

/// A budget definition extracted from a periodic transaction.
#[derive(Debug, Clone, Serialize)]
pub struct Budget {
    pub period: BudgetPeriod,
    pub entries: Vec<BudgetEntry>,
}

/// A single budget line item.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetEntry {
    pub account: String,
    pub amount: Decimal,
    pub commodity: String,
}

/// A row in a budget-vs-actual comparison report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetRow {
    pub account: String,
    pub budget: String,
    pub actual: String,
    pub difference: String,
    pub percentage: String,
    pub commodity: String,
    pub over_budget: bool,
}

/// A data point for budget vs actual chart series.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetSummaryPoint {
    pub period: String,
    pub budgeted: String,
    pub actual: String,
}

/// Parse periodic transactions from a journal into Budget structs.
pub fn extract_budgets(journal: &Journal) -> Vec<Budget> {
    let mut budgets = Vec::new();

    for item in &journal.items {
        if let JournalItem::PeriodicTransaction(pt) = item {
            let period = match parse_period(&pt.period) {
                Some(p) => p,
                None => continue,
            };

            let mut entries = Vec::new();
            for posting in &pt.postings {
                if let Some(ref amt) = posting.amount {
                    entries.push(BudgetEntry {
                        account: posting.account.full.clone(),
                        amount: amt.quantity,
                        commodity: amt.commodity.clone(),
                    });
                }
            }

            if !entries.is_empty() {
                budgets.push(Budget { period, entries });
            }
        }
    }

    budgets
}

/// Parse a period string into a BudgetPeriod enum.
fn parse_period(s: &str) -> Option<BudgetPeriod> {
    let lower = s.to_lowercase();
    if lower.contains("year") || lower == "yearly" || lower == "annually" {
        Some(BudgetPeriod::Yearly)
    } else if lower.contains("quarter") || lower == "quarterly" {
        Some(BudgetPeriod::Quarterly)
    } else if lower.contains("month") || lower == "monthly" || lower == "every month" {
        Some(BudgetPeriod::Monthly)
    } else if lower.contains("week") || lower == "weekly" {
        Some(BudgetPeriod::Weekly)
    } else {
        // Default to monthly for unrecognized periods
        Some(BudgetPeriod::Monthly)
    }
}

/// Calculate how many complete periods fit in a date range.
fn count_periods(period: &BudgetPeriod, date_from: NaiveDate, date_to: NaiveDate) -> Decimal {
    if date_to < date_from {
        return Decimal::ZERO;
    }

    match period {
        BudgetPeriod::Monthly => {
            let months = (date_to.year() - date_from.year()) * 12
                + (date_to.month() as i32 - date_from.month() as i32)
                + 1;
            Decimal::from(months.max(0))
        }
        BudgetPeriod::Quarterly => {
            let from_q = (date_from.month() - 1) / 3;
            let to_q = (date_to.month() - 1) / 3;
            let quarters = (date_to.year() - date_from.year()) * 4
                + (to_q as i32 - from_q as i32)
                + 1;
            Decimal::from(quarters.max(0))
        }
        BudgetPeriod::Yearly => {
            let years = date_to.year() - date_from.year() + 1;
            Decimal::from(years.max(0))
        }
        BudgetPeriod::Weekly => {
            let days = (date_to - date_from).num_days() + 1;
            let weeks = days / 7;
            Decimal::from(weeks.max(1))
        }
    }
}

/// Generate a budget-vs-actual comparison report.
pub fn budget_vs_actual(
    transactions: &[ResolvedTransaction],
    budgets: &[Budget],
    target_commodity: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Vec<BudgetRow> {
    // Default date range: current month if not specified
    let today = chrono::Local::now().date_naive();
    let from = date_from.unwrap_or_else(|| {
        NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap()
    });
    let to = date_to.unwrap_or(today);

    // Compute actual spending per account
    let mut actuals: BTreeMap<String, MixedAmount> = BTreeMap::new();
    for txn in transactions {
        if txn.date < from || txn.date > to {
            continue;
        }
        for posting in &txn.postings {
            let entry = actuals
                .entry(posting.account.full.clone())
                .or_insert_with(MixedAmount::zero);
            entry.add_mixed(&posting.amount);
        }
    }

    // Also accumulate parent account totals for actuals
    let leaf_accounts: Vec<String> = actuals.keys().cloned().collect();
    let mut inclusive_actuals: BTreeMap<String, Decimal> = BTreeMap::new();
    for (account, amt) in &actuals {
        let val = get_primary_value_pub(amt, target_commodity);
        *inclusive_actuals.entry(account.clone()).or_default() += val;
    }
    // Add child values to parent accounts
    for account in &leaf_accounts {
        let val = get_primary_value_pub(actuals.get(account).unwrap(), target_commodity);
        let parts: Vec<&str> = account.split(':').collect();
        for depth in 1..parts.len() {
            let parent = parts[..depth].join(":");
            *inclusive_actuals.entry(parent).or_default() += val;
        }
    }

    let mut rows = Vec::new();

    for budget in budgets {
        let period_count = count_periods(&budget.period, from, to);

        for entry in &budget.entries {
            let budget_amount = entry.amount * period_count;
            let actual_amount = inclusive_actuals.get(&entry.account).copied().unwrap_or(Decimal::ZERO);

            let difference = budget_amount - actual_amount;
            let percentage = if budget_amount.is_zero() {
                if actual_amount.is_zero() {
                    Decimal::ZERO
                } else {
                    Decimal::from(100)
                }
            } else {
                (actual_amount / budget_amount * Decimal::from(100)).round_dp(0)
            };

            let commodity = if entry.commodity.is_empty() {
                target_commodity.to_string()
            } else {
                entry.commodity.clone()
            };

            rows.push(BudgetRow {
                account: entry.account.clone(),
                budget: budget_amount.to_string(),
                actual: actual_amount.to_string(),
                difference: difference.to_string(),
                percentage: format!("{}%", percentage),
                commodity,
                over_budget: actual_amount > budget_amount,
            });
        }
    }

    rows
}

/// Generate monthly budget vs actual summary for charts.
pub fn budget_summary_series(
    transactions: &[ResolvedTransaction],
    budgets: &[Budget],
    target_commodity: &str,
) -> Vec<BudgetSummaryPoint> {
    if transactions.is_empty() || budgets.is_empty() {
        return vec![];
    }

    let first_date = transactions.first().unwrap().date;
    let last_date = transactions.last().unwrap().date;

    let mut points = Vec::new();
    let mut current = NaiveDate::from_ymd_opt(first_date.year(), first_date.month(), 1).unwrap();

    while current <= last_date {
        let month_end = end_of_month(current);

        // Calculate total budget for this month
        let mut total_budget = Decimal::ZERO;
        for budget in budgets {
            let period_count = count_periods(&budget.period, current, month_end);
            for entry in &budget.entries {
                total_budget += entry.amount * period_count;
            }
        }

        // Collect budget account names
        let budget_accounts: Vec<&str> = budgets
            .iter()
            .flat_map(|b| b.entries.iter().map(|e| e.account.as_str()))
            .collect();

        // Calculate total actual spending on budgeted accounts
        let mut total_actual = Decimal::ZERO;
        for txn in transactions {
            if txn.date < current || txn.date > month_end {
                continue;
            }
            for posting in &txn.postings {
                // Check if posting account matches any budget account (including children)
                for ba in &budget_accounts {
                    if posting.account.full == *ba || posting.account.full.starts_with(&format!("{}:", ba)) {
                        total_actual += get_primary_value_pub(&posting.amount, target_commodity);
                        break;
                    }
                }
            }
        }

        points.push(BudgetSummaryPoint {
            period: current.format("%Y-%m").to_string(),
            budgeted: total_budget.to_string(),
            actual: total_actual.to_string(),
        });

        // Next month
        current = if current.month() == 12 {
            NaiveDate::from_ymd_opt(current.year() + 1, 1, 1).unwrap()
        } else {
            NaiveDate::from_ymd_opt(current.year(), current.month() + 1, 1).unwrap()
        };
    }

    points
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

/// Get accounts that have budgets defined.
pub fn budget_accounts(budgets: &[Budget]) -> Vec<String> {
    let mut accounts: Vec<String> = budgets
        .iter()
        .flat_map(|b| b.entries.iter().map(|e| e.account.clone()))
        .collect();
    accounts.sort();
    accounts.dedup();
    accounts
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
    fn extract_monthly_budget() {
        let journal = parse(
            "~ monthly\n    expenses:food  $400.00\n    expenses:rent  $1200.00\n    income\n",
        )
        .unwrap();
        let budgets = extract_budgets(&journal);

        assert_eq!(budgets.len(), 1);
        assert_eq!(budgets[0].period, BudgetPeriod::Monthly);
        assert_eq!(budgets[0].entries.len(), 2);
        assert_eq!(budgets[0].entries[0].account, "expenses:food");
        assert_eq!(budgets[0].entries[0].amount, dec!(400.00));
        assert_eq!(budgets[0].entries[1].account, "expenses:rent");
        assert_eq!(budgets[0].entries[1].amount, dec!(1200.00));
    }

    #[test]
    fn extract_quarterly_budget() {
        let journal = parse(
            "~ quarterly\n    expenses:insurance  $600.00\n    assets:checking\n",
        )
        .unwrap();
        let budgets = extract_budgets(&journal);

        assert_eq!(budgets.len(), 1);
        assert_eq!(budgets[0].period, BudgetPeriod::Quarterly);
    }

    #[test]
    fn budget_times_three_months() {
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();
        let count = count_periods(&BudgetPeriod::Monthly, from, to);
        assert_eq!(count, dec!(3));
    }

    #[test]
    fn budget_vs_actual_exact_match() {
        let input = "~ monthly\n    expenses:food  $400.00\n    income\n\n\
                     2024-01-15 Grocery\n    expenses:food  $400.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let budgets = extract_budgets(&journal);
        let txns = resolve(input);

        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
        let report = budget_vs_actual(&txns, &budgets, "$", Some(from), Some(to));

        assert_eq!(report.len(), 1);
        assert_eq!(report[0].account, "expenses:food");
        assert_eq!(report[0].budget, "400.00");
        assert_eq!(report[0].actual, "400.00");
        assert_eq!(report[0].percentage, "100%");
        assert!(!report[0].over_budget);
    }

    #[test]
    fn budget_vs_actual_over_budget() {
        let input = "~ monthly\n    expenses:food  $400.00\n    income\n\n\
                     2024-01-15 Grocery\n    expenses:food  $500.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let budgets = extract_budgets(&journal);
        let txns = resolve(input);

        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
        let report = budget_vs_actual(&txns, &budgets, "$", Some(from), Some(to));

        assert_eq!(report.len(), 1);
        assert!(report[0].over_budget);
        assert_eq!(report[0].percentage, "125%");
    }

    #[test]
    fn budget_vs_actual_under_budget() {
        let input = "~ monthly\n    expenses:food  $400.00\n    income\n\n\
                     2024-01-15 Grocery\n    expenses:food  $200.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let budgets = extract_budgets(&journal);
        let txns = resolve(input);

        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
        let report = budget_vs_actual(&txns, &budgets, "$", Some(from), Some(to));

        assert_eq!(report.len(), 1);
        assert!(!report[0].over_budget);
        assert_eq!(report[0].difference, "200.00");
    }

    #[test]
    fn budget_multiple_accounts() {
        let input = "~ monthly\n    expenses:food  $400.00\n    expenses:rent  $1200.00\n    income\n\n\
                     2024-01-15 Grocery\n    expenses:food  $350.00\n    assets:checking\n\n\
                     2024-01-01 Rent\n    expenses:rent  $1200.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let budgets = extract_budgets(&journal);
        let txns = resolve(input);

        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
        let report = budget_vs_actual(&txns, &budgets, "$", Some(from), Some(to));

        assert_eq!(report.len(), 2);
        let food = report.iter().find(|r| r.account == "expenses:food").unwrap();
        assert_eq!(food.actual, "350.00");
        let rent = report.iter().find(|r| r.account == "expenses:rent").unwrap();
        assert_eq!(rent.actual, "1200.00");
        assert!(!rent.over_budget);
    }

    #[test]
    fn budget_accounts_list() {
        let journal = parse(
            "~ monthly\n    expenses:food  $400.00\n    expenses:rent  $1200.00\n    income\n",
        )
        .unwrap();
        let budgets = extract_budgets(&journal);
        let accounts = budget_accounts(&budgets);
        assert_eq!(accounts, vec!["expenses:food", "expenses:rent"]);
    }

    #[test]
    fn count_periods_yearly() {
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
        assert_eq!(count_periods(&BudgetPeriod::Yearly, from, to), dec!(1));
    }

    #[test]
    fn count_periods_quarterly() {
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();
        assert_eq!(count_periods(&BudgetPeriod::Quarterly, from, to), dec!(2));
    }
}
