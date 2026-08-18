//! Forecast: materialize periodic transactions (`~ PERIODEXPR`) into future
//! transactions, like hledger --forecast. Generated transactions start the
//! day after the last real transaction and run to the requested horizon.

use chrono::{Duration, Months, NaiveDate};

use hledger_parser::ast::{Journal, JournalItem, SourceSpan, Status, Transaction};

use crate::balance::{resolve_transactions, ResolvedTransaction};
use crate::budget::{parse_period_expression, PeriodSpec, PeriodUnit};

/// Default forecast horizon when no explicit end date is given (hledger
/// defaults to six months from today; we anchor on the journal instead so
/// results don't depend on the wall clock).
pub fn default_horizon(last_real: NaiveDate) -> NaiveDate {
    last_real
        .checked_add_months(Months::new(6))
        .unwrap_or(last_real)
}

fn align(date: NaiveDate, unit: PeriodUnit) -> NaiveDate {
    use chrono::Datelike;
    match unit {
        PeriodUnit::Day => date,
        PeriodUnit::Week => {
            date - Duration::days(date.weekday().num_days_from_monday() as i64)
        }
        PeriodUnit::Month => NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap(),
        PeriodUnit::Quarter => {
            let q_month = ((date.month() - 1) / 3) * 3 + 1;
            NaiveDate::from_ymd_opt(date.year(), q_month, 1).unwrap()
        }
        PeriodUnit::Year => NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap(),
    }
}

fn step(date: NaiveDate, unit: PeriodUnit, every: u32) -> Option<NaiveDate> {
    match unit {
        PeriodUnit::Day => date.checked_add_signed(Duration::days(every as i64)),
        PeriodUnit::Week => date.checked_add_signed(Duration::weeks(every as i64)),
        PeriodUnit::Month => date.checked_add_months(Months::new(every)),
        PeriodUnit::Quarter => date.checked_add_months(Months::new(3 * every)),
        PeriodUnit::Year => date.checked_add_months(Months::new(12 * every)),
    }
}

/// Occurrence dates of a period spec in (from, to] — forecast starts strictly
/// after the last real transaction.
fn occurrences(spec: &PeriodSpec, after: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    let anchor = spec.start.unwrap_or_else(|| align(after, spec.unit));
    let mut dates = Vec::new();
    let mut current = anchor;
    let mut guard = 0u32;

    while current <= after {
        match step(current, spec.unit, spec.every) {
            Some(next) => current = next,
            None => return dates,
        }
        guard += 1;
        if guard > 100_000 {
            return dates;
        }
    }

    while current <= to {
        if let Some(end) = spec.end {
            if current >= end {
                break;
            }
        }
        dates.push(current);
        match step(current, spec.unit, spec.every) {
            Some(next) => current = next,
            None => break,
        }
        guard += 1;
        if guard > 100_000 {
            break;
        }
    }

    dates
}

/// Generate resolved forecast transactions from the journal's periodic rules,
/// covering (after_date, horizon]. Every generated posting is flagged
/// `generated` so the UI can distinguish projections from bookkept facts.
pub fn forecast_transactions(
    journal: &Journal,
    after_date: NaiveDate,
    horizon: NaiveDate,
) -> Vec<ResolvedTransaction> {
    let mut generated_ast: Vec<JournalItem> = Vec::new();

    for item in &journal.items {
        let JournalItem::PeriodicTransaction(pt) = item else {
            continue;
        };
        let Some(spec) = parse_period_expression(&pt.period) else {
            continue; // already warned at budget extraction
        };
        // Periodic transactions used purely as budget goals often have no
        // meaningful description; still forecast them like hledger does.
        for date in occurrences(&spec, after_date, horizon) {
            generated_ast.push(JournalItem::Transaction(Transaction {
                span: SourceSpan { start: 0, end: 0, line: pt.span.line },
                date,
                secondary_date: None,
                status: Status::Unmarked,
                code: None,
                description: if pt.description.is_empty() {
                    "Forecast".to_string()
                } else {
                    pt.description.clone()
                },
                comment: None,
                tags: vec![],
                postings: pt.postings.clone(),
            }));
        }
    }

    if generated_ast.is_empty() {
        return vec![];
    }

    let temp = Journal {
        items: generated_ast,
        source_path: None,
        warnings: vec![],
    };

    match resolve_transactions(&temp) {
        Ok(mut txns) => {
            for txn in &mut txns {
                for posting in &mut txn.postings {
                    posting.generated = true;
                }
            }
            txns
        }
        // A periodic rule that doesn't balance can't be forecast; budgets
        // already surface this as a warning.
        Err(_) => vec![],
    }
}

/// Real transactions plus forecast, sorted by date.
pub fn with_forecast(
    journal: &Journal,
    real: &[ResolvedTransaction],
    horizon: Option<NaiveDate>,
) -> Vec<ResolvedTransaction> {
    let Some(last_real) = real.last().map(|t| t.date) else {
        return real.to_vec();
    };
    let horizon = horizon.unwrap_or_else(|| default_horizon(last_real));
    if horizon <= last_real {
        return real.to_vec();
    }

    let mut all = real.to_vec();
    all.extend(forecast_transactions(journal, last_real, horizon));
    all.sort_by_key(|t| t.date);
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use hledger_parser::parse;
    use rust_decimal_macros::dec;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn monthly_rule_materializes_after_last_real_txn() {
        let input = "\
~ monthly  Rent
    expenses:rent  $1200.00
    assets:checking

2024-01-05 Seed
    assets:checking  $5000.00
    equity:opening
";
        let journal = parse(input).unwrap();
        let real = resolve_transactions(&journal).unwrap();
        let all = with_forecast(&journal, &real, Some(d(2024, 4, 30)));

        let forecast: Vec<&ResolvedTransaction> = all
            .iter()
            .filter(|t| t.postings.iter().any(|p| p.generated))
            .collect();
        // Feb 1, Mar 1, Apr 1.
        assert_eq!(forecast.len(), 3);
        assert_eq!(forecast[0].date, d(2024, 2, 1));
        assert_eq!(forecast[0].description, "Rent");
        assert_eq!(forecast[0].postings[0].amount.get("$"), dec!(1200));
        // Elided balancing posting inferred.
        assert_eq!(forecast[0].postings[1].amount.get("$"), dec!(-1200));
    }

    #[test]
    fn forecast_respects_period_bounds() {
        let input = "\
~ monthly from 2024-03 to 2024-05  Bounded
    expenses:sub  $10.00
    assets:cash

2024-01-05 Seed
    assets:cash  $100.00
    equity:opening
";
        let journal = parse(input).unwrap();
        let real = resolve_transactions(&journal).unwrap();
        let all = with_forecast(&journal, &real, Some(d(2024, 12, 31)));
        let dates: Vec<NaiveDate> = all
            .iter()
            .filter(|t| t.postings.iter().any(|p| p.generated))
            .map(|t| t.date)
            .collect();
        // March and April only ('to' is exclusive).
        assert_eq!(dates, vec![d(2024, 3, 1), d(2024, 4, 1)]);
    }

    #[test]
    fn no_periodic_rules_no_forecast() {
        let input = "2024-01-05 Seed\n    assets:cash  $100.00\n    equity:opening\n";
        let journal = parse(input).unwrap();
        let real = resolve_transactions(&journal).unwrap();
        let all = with_forecast(&journal, &real, Some(d(2024, 12, 31)));
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn every_two_weeks() {
        let input = "\
~ every 2 weeks  Paycheck
    assets:checking  $2000.00
    income:salary

2024-01-31 Seed
    assets:checking  $1.00
    equity:opening
";
        let journal = parse(input).unwrap();
        let real = resolve_transactions(&journal).unwrap();
        let all = with_forecast(&journal, &real, Some(d(2024, 2, 29)));
        let dates: Vec<NaiveDate> = all
            .iter()
            .filter(|t| t.postings.iter().any(|p| p.generated))
            .map(|t| t.date)
            .collect();
        // Anchored to the Monday-aligned grid; two occurrences in February.
        assert_eq!(dates.len(), 2, "dates: {:?}", dates);
        assert_eq!((dates[1] - dates[0]).num_days(), 14);
    }
}
