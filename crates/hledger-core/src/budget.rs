use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::amount::MixedAmount;
use crate::balance::ResolvedTransaction;
use crate::price_db::PriceDb;
use crate::reports::valued_quantity;

use hledger_parser::ast::{Journal, JournalItem};

// Period expressions live in one place so budgets and forecasts can't drift
// apart; re-exported because callers have long imported them from here.
pub use crate::period::{
    count_occurrences, parse_period_expression, PeriodSpec, PeriodUnit,
};

/// A budget definition extracted from a periodic transaction.
#[derive(Debug, Clone, Serialize)]
pub struct Budget {
    pub period: PeriodSpec,
    pub description: String,
    pub entries: Vec<BudgetEntry>,
    /// Line of the periodic transaction in the source (for editing).
    pub line: usize,
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
    /// True for income-style goals (negative budget amounts): "over budget"
    /// then means the goal was missed, not exceeded.
    pub is_income: bool,
}

/// A data point for budget vs actual chart series.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetSummaryPoint {
    pub period: String,
    pub budgeted: String,
    pub actual: String,
}

pub struct BudgetExtraction {
    pub budgets: Vec<Budget>,
    pub warnings: Vec<String>,
}

/// Parse periodic transactions from a journal into Budget structs.
pub fn extract_budgets(journal: &Journal) -> Vec<Budget> {
    extract_budgets_with_warnings(journal).budgets
}

pub fn extract_budgets_with_warnings(journal: &Journal) -> BudgetExtraction {
    let mut budgets = Vec::new();
    let mut warnings = Vec::new();

    for item in &journal.items {
        if let JournalItem::PeriodicTransaction(pt) = item {
            let period = match parse_period_expression(&pt.period) {
                Some(p) => p,
                None => {
                    warnings.push(format!(
                        "line {}: unsupported period expression '~ {}' — this budget is ignored (supported: daily/weekly/monthly/quarterly/yearly, 'every N <unit>s', with optional 'from DATE' / 'to DATE')",
                        pt.span.line, pt.period
                    ));
                    continue;
                }
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
                budgets.push(Budget {
                    period,
                    description: pt.description.clone(),
                    entries,
                    line: pt.span.line,
                });
            }
        }
    }

    BudgetExtraction { budgets, warnings }
}








/// Generate a budget-vs-actual comparison report. Goals from multiple budgets
/// for the same account+commodity are merged into one row (hledger behavior);
/// actuals are valued into the entry's commodity via the price database.
pub fn budget_vs_actual(
    transactions: &[ResolvedTransaction],
    budgets: &[Budget],
    price_db: &PriceDb,
    target_commodity: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Vec<BudgetRow> {
    let from = date_from.unwrap_or_else(|| {
        transactions
            .first()
            .map(|t| t.date)
            .unwrap_or_else(|| chrono::Local::now().date_naive())
    });
    let to = date_to.unwrap_or_else(|| {
        transactions
            .last()
            .map(|t| t.date)
            .unwrap_or_else(|| chrono::Local::now().date_naive())
    });

    // Actual amounts per account (inclusive of children), kept as MixedAmount
    // so valuation is commodity-correct.
    let mut actuals: BTreeMap<String, MixedAmount> = BTreeMap::new();
    for txn in transactions {
        for posting in &txn.postings {
            if posting.date < from || posting.date > to {
                continue;
            }
            actuals
                .entry(posting.account.full.clone())
                .or_insert_with(MixedAmount::zero)
                .add_mixed(&posting.amount);
        }
    }
    // Roll leaf actuals up into parents.
    let leaves: Vec<(String, MixedAmount)> =
        actuals.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (account, amt) in &leaves {
        let parts: Vec<&str> = account.split(':').collect();
        for depth in 1..parts.len() {
            let parent = parts[..depth].join(":");
            actuals
                .entry(parent)
                .or_insert_with(MixedAmount::zero)
                .add_mixed(amt);
        }
    }

    // Merge goals per (account, commodity).
    let mut goals: BTreeMap<(String, String), Decimal> = BTreeMap::new();
    for budget in budgets {
        let occurrences = count_occurrences(&budget.period, from, to);
        if occurrences == 0 {
            continue;
        }
        for entry in &budget.entries {
            let commodity = if entry.commodity.is_empty() {
                target_commodity.to_string()
            } else {
                entry.commodity.clone()
            };
            *goals
                .entry((entry.account.clone(), commodity))
                .or_insert(Decimal::ZERO) += entry.amount * Decimal::from(occurrences);
        }
    }

    let mut rows = Vec::new();
    for ((account, commodity), budget_amount) in goals {
        let actual_mixed = actuals.get(&account).cloned().unwrap_or_default();
        let (actual_amount, _skipped) =
            valued_quantity(&actual_mixed, &commodity, price_db, to);

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

        let is_income = budget_amount < Decimal::ZERO;

        rows.push(BudgetRow {
            account,
            budget: budget_amount.to_string(),
            actual: actual_amount.to_string(),
            difference: difference.to_string(),
            percentage: format!("{}%", percentage),
            commodity,
            // "Worse than goal" in both directions: spent more than an
            // expense goal, or earned less than an income goal.
            over_budget: actual_amount > budget_amount
                && !is_income
                || (is_income && actual_amount > budget_amount),
            is_income,
        });
    }

    rows
}

/// Monthly budget-vs-actual chart series. Goals are occurrence-based: a
/// yearly budget contributes only in the month containing its occurrence,
/// not 12x across the year. Respects the requested date range.
pub fn budget_summary_series(
    transactions: &[ResolvedTransaction],
    budgets: &[Budget],
    price_db: &PriceDb,
    target_commodity: &str,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Vec<BudgetSummaryPoint> {
    if transactions.is_empty() || budgets.is_empty() {
        return vec![];
    }

    let first_date = date_from.unwrap_or_else(|| transactions.first().unwrap().date);
    let last_date = date_to.unwrap_or_else(|| transactions.last().unwrap().date);

    let budget_accounts: Vec<&str> = budgets
        .iter()
        .flat_map(|b| b.entries.iter().map(|e| e.account.as_str()))
        .collect();

    let mut points = Vec::new();
    let mut current = NaiveDate::from_ymd_opt(first_date.year(), first_date.month(), 1).unwrap();

    while current <= last_date {
        let month_end = end_of_month(current);
        let bucket_from = current.max(first_date);
        let bucket_to = month_end.min(last_date);

        // Goal: occurrences of each budget starting in this month.
        let mut total_budget = Decimal::ZERO;
        for budget in budgets {
            let occurrences = count_occurrences(&budget.period, current, month_end);
            if occurrences == 0 {
                continue;
            }
            for entry in &budget.entries {
                let goal = entry.amount * Decimal::from(occurrences);
                let commodity = if entry.commodity.is_empty() {
                    target_commodity
                } else {
                    &entry.commodity
                };
                if commodity == target_commodity {
                    total_budget += goal;
                } else if let Some(converted) =
                    price_db.convert(goal, commodity, target_commodity, month_end)
                {
                    total_budget += converted;
                }
            }
        }

        // Actual spending on budgeted accounts within the clamped bucket.
        let mut actual_mixed = MixedAmount::zero();
        for txn in transactions {
            for posting in &txn.postings {
                if posting.date < bucket_from || posting.date > bucket_to {
                    continue;
                }
                for ba in &budget_accounts {
                    if posting.account.full == *ba
                        || posting.account.full.starts_with(&format!("{}:", ba))
                    {
                        actual_mixed.add_mixed(&posting.amount);
                        break;
                    }
                }
            }
        }
        let (total_actual, _) =
            valued_quantity(&actual_mixed, target_commodity, price_db, month_end);

        points.push(BudgetSummaryPoint {
            period: current.format("%Y-%m").to_string(),
            budgeted: total_budget.to_string(),
            actual: total_actual.to_string(),
        });

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

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn db() -> PriceDb {
        PriceDb::new()
    }

    #[test]
    fn extract_monthly_budget() {
        let journal = parse(
            "~ monthly\n    expenses:food  $400.00\n    expenses:rent  $1200.00\n    income\n",
        )
        .unwrap();
        let budgets = extract_budgets(&journal);

        assert_eq!(budgets.len(), 1);
        assert_eq!(budgets[0].period.unit, PeriodUnit::Month);
        assert_eq!(budgets[0].entries.len(), 2);
        assert_eq!(budgets[0].entries[0].account, "expenses:food");
        assert_eq!(budgets[0].entries[0].amount, dec!(400.00));
    }

    #[test]
    fn parse_every_n_weeks() {
        let spec = parse_period_expression("every 2 weeks").unwrap();
        assert_eq!(spec.unit, PeriodUnit::Week);
        assert_eq!(spec.every, 2);
    }

    #[test]
    fn parse_monthly_from() {
        let spec = parse_period_expression("monthly from 2026-03").unwrap();
        assert_eq!(spec.unit, PeriodUnit::Month);
        assert_eq!(spec.start, Some(d(2026, 3, 1)));
    }

    #[test]
    fn unknown_period_is_rejected_with_warning_not_monthly() {
        let journal = parse(
            "~ every 3rd thursday  Odd budget\n    expenses:x  $10\n    assets:cash\n",
        )
        .unwrap();
        let extraction = extract_budgets_with_warnings(&journal);
        assert!(extraction.budgets.is_empty());
        assert_eq!(extraction.warnings.len(), 1);
        assert!(extraction.warnings[0].contains("unsupported period expression"));
    }

    #[test]
    fn occurrences_partial_range_not_calendar_touched() {
        // Jan-15..Feb-5 contains exactly ONE monthly occurrence (Feb 1).
        let spec = parse_period_expression("monthly").unwrap();
        assert_eq!(count_occurrences(&spec, d(2026, 1, 15), d(2026, 2, 5)), 1);
        // A 1-day range mid-month contains none.
        assert_eq!(count_occurrences(&spec, d(2026, 1, 15), d(2026, 1, 15)), 0);
        // A full month contains one.
        assert_eq!(count_occurrences(&spec, d(2026, 1, 1), d(2026, 1, 31)), 1);
        // Three full months contain three.
        assert_eq!(count_occurrences(&spec, d(2026, 1, 1), d(2026, 3, 31)), 3);
    }

    #[test]
    fn occurrences_every_two_weeks() {
        let spec = parse_period_expression("every 2 weeks").unwrap();
        // January 2026: aligned to Monday Dec 29; occurrences Jan 12, Jan 26.
        let n = count_occurrences(&spec, d(2026, 1, 1), d(2026, 1, 31));
        assert_eq!(n, 2);
    }

    #[test]
    fn occurrences_respect_from_bound() {
        let spec = parse_period_expression("monthly from 2026-03").unwrap();
        assert_eq!(count_occurrences(&spec, d(2026, 1, 1), d(2026, 1, 31)), 0);
        assert_eq!(count_occurrences(&spec, d(2026, 3, 1), d(2026, 3, 31)), 1);
    }

    #[test]
    fn occurrences_yearly_only_in_january() {
        let spec = parse_period_expression("yearly").unwrap();
        assert_eq!(count_occurrences(&spec, d(2026, 1, 1), d(2026, 1, 31)), 1);
        assert_eq!(count_occurrences(&spec, d(2026, 6, 1), d(2026, 6, 30)), 0);
    }

    #[test]
    fn budget_vs_actual_exact_match() {
        let input = "~ monthly\n    expenses:food  $400.00\n    income\n\n\
                     2024-01-15 Grocery\n    expenses:food  $400.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let budgets = extract_budgets(&journal);
        let txns = resolve(input);

        let report =
            budget_vs_actual(&txns, &budgets, &db(), "$", Some(d(2024, 1, 1)), Some(d(2024, 1, 31)));

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

        let report =
            budget_vs_actual(&txns, &budgets, &db(), "$", Some(d(2024, 1, 1)), Some(d(2024, 1, 31)));

        assert!(report[0].over_budget);
        assert_eq!(report[0].percentage, "125%");
    }

    #[test]
    fn duplicate_budgets_merge_into_one_row() {
        // Two ~ blocks for the same account: goals merge, one row (hledger).
        let input = "~ monthly\n    expenses:food  $100.00\n    income\n\n\
                     ~ monthly\n    expenses:food  $150.00\n    income\n\n\
                     2024-01-15 G\n    expenses:food  $80.00\n    assets:cash\n";
        let journal = parse(input).unwrap();
        let budgets = extract_budgets(&journal);
        let txns = resolve(input);

        let report =
            budget_vs_actual(&txns, &budgets, &db(), "$", Some(d(2024, 1, 1)), Some(d(2024, 1, 31)));
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].budget, "250.00");
        assert_eq!(report[0].actual, "80.00");
    }

    #[test]
    fn income_budget_signs_coherent() {
        let input = "~ monthly\n    income:salary  $-3000.00\n    expenses\n\n\
                     2024-01-15 Pay\n    assets:bank  $3200.00\n    income:salary  $-3200.00\n";
        let journal = parse(input).unwrap();
        let budgets = extract_budgets(&journal);
        let txns = resolve(input);

        let report =
            budget_vs_actual(&txns, &budgets, &db(), "$", Some(d(2024, 1, 1)), Some(d(2024, 1, 31)));
        let salary = report.iter().find(|r| r.account == "income:salary").unwrap();
        assert!(salary.is_income);
        // Earned MORE than the goal: 107%, and not flagged as "over budget".
        assert_eq!(salary.percentage, "107%");
        assert!(!salary.over_budget);
    }

    #[test]
    fn income_budget_missed_goal_flagged() {
        let input = "~ monthly\n    income:salary  $-3000.00\n    expenses\n\n\
                     2024-01-15 Pay\n    assets:bank  $2000.00\n    income:salary  $-2000.00\n";
        let journal = parse(input).unwrap();
        let budgets = extract_budgets(&journal);
        let txns = resolve(input);

        let report =
            budget_vs_actual(&txns, &budgets, &db(), "$", Some(d(2024, 1, 1)), Some(d(2024, 1, 31)));
        let salary = report.iter().find(|r| r.account == "income:salary").unwrap();
        assert!(salary.over_budget, "missed income goal should be flagged");
    }

    #[test]
    fn yearly_budget_not_charged_every_month_in_series() {
        let input = "~ yearly\n    expenses:insurance  $1200.00\n    assets:cash\n\n\
                     2024-01-05 Ins\n    expenses:insurance  $1200.00\n    assets:cash\n\n\
                     2024-06-05 Other\n    expenses:other  $10.00\n    assets:cash\n";
        let journal = parse(input).unwrap();
        let budgets = extract_budgets(&journal);
        let txns = resolve(input);

        let series = budget_summary_series(&txns, &budgets, &db(), "$", None, None);
        // January: the yearly occurrence.
        assert_eq!(series[0].budgeted, "1200.00");
        // June: no occurrence, budget 0.
        let june = series.iter().find(|p| p.period == "2024-06").unwrap();
        assert_eq!(june.budgeted, "0");
    }

    #[test]
    fn summary_series_respects_date_range() {
        let input = "~ monthly\n    expenses:food  $100.00\n    income\n\n\
                     2024-01-15 A\n    expenses:food  $50.00\n    assets:cash\n\n\
                     2024-03-15 B\n    expenses:food  $70.00\n    assets:cash\n";
        let journal = parse(input).unwrap();
        let budgets = extract_budgets(&journal);
        let txns = resolve(input);

        let series = budget_summary_series(
            &txns, &budgets, &db(), "$",
            Some(d(2024, 3, 1)), Some(d(2024, 3, 31)),
        );
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].period, "2024-03");
        assert_eq!(series[0].actual, "70.00");
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
    fn audit_sample_budget_journal() {
        let text = std::fs::read_to_string("../../tests/fixtures/sample-with-budget.journal").unwrap();
        let journal = hledger_parser::parse(&text).expect("parse failed");
        let budgets = extract_budgets(&journal);
        assert_eq!(budgets.len(), 1, "Should find 1 budget");
        assert_eq!(budgets[0].period.unit, PeriodUnit::Month);
        assert_eq!(budgets[0].entries.len(), 6, "Should have 6 budget entries");

        let txns = crate::balance::resolve_transactions(&journal).expect("resolve failed");
        let report = budget_vs_actual(
            &txns, &budgets, &db(), "$",
            Some(d(2026, 1, 1)), Some(d(2026, 1, 31)),
        );

        let rent = report.iter().find(|r| r.account == "expenses:rent").unwrap();
        assert_eq!(rent.percentage, "100%", "Rent should be exactly on budget");
        assert!(!rent.over_budget);

        let utilities = report.iter().find(|r| r.account == "expenses:utilities").unwrap();
        assert!(utilities.over_budget, "Utilities should be over budget");
    }
}
