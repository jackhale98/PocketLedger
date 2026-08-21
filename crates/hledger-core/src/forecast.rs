//! Forecast: materialize periodic transactions (`~ PERIODEXPR`) into future
//! transactions, like `hledger --forecast`.
//!
//! Window semantics match hledger: with no explicit period, forecasting runs
//! from the day after the last ordinary transaction to six months out, and an
//! unanchored rule like `~ monthly` inherits the window's start day (see
//! [`crate::period`]).
//!
//! One deliberate deviation: hledger resolves forecast transactions inside the
//! main journal, so a generated entry landing before a balance assertion can
//! make that assertion fail. We resolve them separately, keeping a forecast a
//! view over the books rather than a rewrite of them.

use chrono::{Months, NaiveDate};

use hledger_parser::ast::{Journal, JournalItem, SourceSpan, Status, Transaction};

use crate::balance::{resolve_transactions, ResolvedTransaction};
use crate::period::{forecast_occurrences, parse_period_expression};

/// Default forecast horizon: six months past `from`. hledger measures this
/// from today; we measure from the journal so results don't shift with the
/// wall clock (callers pass today when they want hledger's exact behavior).
pub fn default_horizon(from: NaiveDate) -> NaiveDate {
    from.checked_add_months(Months::new(6)).unwrap_or(from)
}

/// Generate resolved forecast transactions from the journal's periodic rules,
/// covering the inclusive window `[window_start, horizon]`. Every generated
/// posting is flagged `generated` so the UI can distinguish projections from
/// bookkept facts.
pub fn forecast_transactions(
    journal: &Journal,
    window_start: NaiveDate,
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
        for date in forecast_occurrences(&spec, window_start, horizon) {
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

/// The window hledger forecasts over by default: from the day after the last
/// ordinary transaction, to six months out. Returns None when there is
/// nothing to forecast.
pub fn default_window(
    real: &[ResolvedTransaction],
    horizon: Option<NaiveDate>,
) -> Option<(NaiveDate, NaiveDate)> {
    let last_real = real.iter().map(|t| t.date).max()?;
    let start = last_real.succ_opt()?;
    let horizon = horizon.unwrap_or_else(|| default_horizon(last_real));
    (horizon >= start).then_some((start, horizon))
}

/// Real transactions plus forecast, sorted by date.
pub fn with_forecast(
    journal: &Journal,
    real: &[ResolvedTransaction],
    horizon: Option<NaiveDate>,
) -> Vec<ResolvedTransaction> {
    let Some((start, horizon)) = default_window(real, horizon) else {
        return real.to_vec();
    };

    let mut all = real.to_vec();
    all.extend(forecast_transactions(journal, start, horizon));
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
        // Verified against hledger 1.50.3:
        //   hledger print --forecast=2024-01-06..2024-05-01
        //   -> Rent on 2024-01-06, 02-06, 03-06, 04-06
        // The rule has no `from`, so it inherits the window's start day (the
        // day after the last real transaction) rather than the calendar 1st.
        assert_eq!(
            forecast.iter().map(|t| t.date).collect::<Vec<_>>(),
            vec![d(2024, 1, 6), d(2024, 2, 6), d(2024, 3, 6), d(2024, 4, 6)]
        );
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
        // Verified against hledger 1.50.3:
        //   hledger print --forecast=2024-02-01..2024-03-01
        //   -> Paycheck on 2024-02-01, 02-15, 02-29
        assert_eq!(dates, vec![d(2024, 2, 1), d(2024, 2, 15), d(2024, 2, 29)]);
    }
}
