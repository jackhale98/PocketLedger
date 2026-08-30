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

/// Forecast output plus anything that stopped a rule from contributing.
pub struct ForecastOutcome {
    pub transactions: Vec<ResolvedTransaction>,
    /// (source line, reason) for each rule that generated nothing.
    pub errors: Vec<(usize, String)>,
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
    forecast_checked(journal, window_start, horizon).transactions
}

/// As [`forecast_transactions`], but reporting why any rule contributed
/// nothing.
///
/// Each rule is resolved on its own. Resolving them together meant a single
/// unbalanced rule failed the whole batch, so one bad rule silently emptied
/// the entire forecast. hledger hard-errors on such a rule under `--forecast`;
/// an editor can't refuse to open the file, so it forecasts what it can and
/// reports the rest.
pub fn forecast_checked(
    journal: &Journal,
    window_start: NaiveDate,
    horizon: NaiveDate,
) -> ForecastOutcome {
    let mut transactions = Vec::new();
    let mut errors = Vec::new();

    for item in &journal.items {
        let JournalItem::PeriodicTransaction(pt) = item else {
            continue;
        };
        let Some(spec) = parse_period_expression(&pt.period) else {
            errors.push((
                pt.span.line,
                format!("Unsupported period expression '{}'.", pt.period),
            ));
            continue;
        };

        let dates = forecast_occurrences(&spec, window_start, horizon);
        if dates.is_empty() {
            continue;
        }

        // Periodic transactions used purely as budget goals often have no
        // meaningful description; still forecast them like hledger does.
        let generated: Vec<JournalItem> = dates
            .into_iter()
            .map(|date| {
                JournalItem::Transaction(Transaction {
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
                })
            })
            .collect();

        let temp = Journal {
            items: generated,
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
                transactions.append(&mut txns);
            }
            Err(e) => errors.push((
                pt.span.line,
                format!("This rule doesn't balance, so it generates nothing ({e})."),
            )),
        }
    }

    transactions.sort_by_key(|t| t.date);
    ForecastOutcome { transactions, errors }
}

/// The window `hledger --forecast` covers with no report start: from the day
/// after the last ordinary transaction to six months out. Returns None when
/// there is nothing to forecast.
pub fn default_window(
    real: &[ResolvedTransaction],
    horizon: Option<NaiveDate>,
) -> Option<(NaiveDate, NaiveDate)> {
    let last_real = real.iter().map(|t| t.date).max()?;
    let start = last_real.succ_opt()?;
    let horizon = horizon.unwrap_or_else(|| default_horizon(last_real));
    (horizon >= start).then_some((start, horizon))
}

/// The window a user-facing projection covers, equivalent to
/// `hledger --forecast -b <today>`.
///
/// hledger computes `forecastStart = max(journalEnd + 1, reportStart)`, so
/// asking it to report from today gives exactly this: the day after the last
/// transaction for a journal that is up to date, and today for one that is
/// stale. [`default_window`] is the same rule with no report start, which is
/// what a bare `--forecast` does; that replays every month since the journal
/// was last touched, which is right for the CLI flag but useless as a "what
/// happens from here" projection.
///
/// Verified against hledger 1.50.3 — see the `projection_matches_hledger_from_today`
/// differential test.
pub fn projection_window(
    real: &[ResolvedTransaction],
    today: NaiveDate,
    horizon: Option<NaiveDate>,
) -> Option<(NaiveDate, NaiveDate)> {
    let start = match real.iter().map(|t| t.date).max() {
        Some(last) => last.succ_opt()?.max(today),
        None => today,
    };
    let horizon = horizon.unwrap_or_else(|| default_horizon(today));
    (horizon >= start).then_some((start, horizon))
}

/// Real transactions plus a projection running from `today` to the horizon.
pub fn with_projection(
    journal: &Journal,
    real: &[ResolvedTransaction],
    today: NaiveDate,
    horizon: Option<NaiveDate>,
) -> Vec<ResolvedTransaction> {
    let Some((start, horizon)) = projection_window(real, today, horizon) else {
        return real.to_vec();
    };
    let mut all = real.to_vec();
    all.extend(forecast_transactions(journal, start, horizon));
    all.sort_by_key(|t| t.date);
    all
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

/// A recurring transaction rule (`~ PERIODEXPR  DESCRIPTION`) as the user
/// manages it. The same rules also drive budget goals — hledger has no way to
/// mark a rule as one or the other, so the UI shows them together.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastRule {
    /// Full period expression, e.g. "monthly from 2026-01".
    pub period: String,
    pub description: String,
    /// Source line of the `~` rule; pass back to replace or delete it.
    pub line: usize,
    /// Which of the journal's files the rule lives in (0 = main file). Line
    /// numbers alone are ambiguous once includes are involved; pass this back
    /// together with `line`.
    pub file_index: usize,
    pub postings: Vec<ForecastPosting>,
    /// Set when the period expression can't be honored; the rule generates
    /// nothing and the message says why.
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastPosting {
    pub account: String,
    /// None for an elided amount (hledger infers it to balance the rule).
    pub amount: Option<String>,
    pub commodity: String,
}

/// Every periodic rule in the journal, in source order. Unparseable ones are
/// included with an `error` rather than dropped, so the UI can offer to fix
/// them instead of silently losing them.
pub fn extract_rules(journal: &Journal) -> Vec<ForecastRule> {
    extract_rules_with_files(journal, &[])
}

/// As [`extract_rules`], with `item_files` mapping each journal item to the
/// index of the file it came from (parallel to `journal.items`). Items
/// beyond the slice are attributed to file 0.
pub fn extract_rules_with_files(journal: &Journal, item_files: &[usize]) -> Vec<ForecastRule> {
    let mut rules = Vec::new();
    for (item_idx, item) in journal.items.iter().enumerate() {
        let JournalItem::PeriodicTransaction(pt) = item else {
            continue;
        };
        let error = parse_period_expression(&pt.period).is_none().then(|| {
            format!(
                "Unsupported period expression '{}' — this rule generates nothing.",
                pt.period
            )
        });
        rules.push(ForecastRule {
            period: pt.period.clone(),
            description: pt.description.clone(),
            line: pt.span.line,
            file_index: item_files.get(item_idx).copied().unwrap_or(0),
            postings: pt
                .postings
                .iter()
                .map(|p| ForecastPosting {
                    account: p.account.full.clone(),
                    amount: p.amount.as_ref().map(|a| a.quantity.to_string()),
                    commodity: p
                        .amount
                        .as_ref()
                        .map(|a| a.commodity.clone())
                        .unwrap_or_default(),
                })
                .collect(),
            error,
        });
    }
    rules
}

/// One period of a cash-flow projection.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionPoint {
    /// Period label, e.g. "2026-03".
    pub period: String,
    /// Money in during the period.
    pub inflow: String,
    /// Money out during the period (positive).
    pub outflow: String,
    /// Balance at the end of the period.
    pub closing: String,
    /// True once the period contains projected rather than recorded activity.
    pub projected: bool,
}

/// A projected cash-flow shortfall.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortfallAlert {
    pub date: String,
    pub balance: String,
    pub description: String,
}

/// Which accounts a projection covers.
pub enum AccountSelector<'a> {
    /// A specific account and its subaccounts.
    Prefix(&'a str),
    /// Every asset/cash account, by declared `type:` or name inference. Used
    /// when no account is chosen, so journals whose asset tree isn't spelled
    /// "assets" still project correctly.
    Assets(&'a crate::classify::AccountClassifier),
}

impl AccountSelector<'_> {
    fn matches(&self, account: &str) -> bool {
        match self {
            Self::Prefix(prefix) => crate::reports::account_matches_prefix(account, prefix),
            Self::Assets(classifier) => classifier.classify(account).is_asset(),
        }
    }
}

/// Monthly cash-flow projection for the accounts matching `selector`,
/// valued in `commodity`. Periods up to and including the last recorded
/// transaction are actuals; later ones are projections.
pub fn cash_flow_projection(
    all: &[ResolvedTransaction],
    last_real: Option<NaiveDate>,
    selector: &AccountSelector<'_>,
    commodity: &str,
    price_db: &crate::price_db::PriceDb,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> Vec<ProjectionPoint> {
    use crate::reports::valued_quantity;
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;

    // period start -> (inflow, outflow); opening carries everything earlier.
    let mut buckets: BTreeMap<NaiveDate, (Decimal, Decimal)> = BTreeMap::new();
    let mut opening = Decimal::ZERO;

    for txn in all {
        for posting in &txn.postings {
            if !selector.matches(&posting.account.full) {
                continue;
            }
            if let Some(to) = to {
                if posting.date > to {
                    continue;
                }
            }
            let (value, _unconvertible) =
                valued_quantity(&posting.amount, commodity, price_db, posting.date);
            if value.is_zero() {
                continue;
            }
            if from.is_some_and(|f| posting.date < f) {
                opening += value;
                continue;
            }
            let key = month_start(posting.date);
            let entry = buckets.entry(key).or_insert((Decimal::ZERO, Decimal::ZERO));
            if value.is_sign_negative() {
                entry.1 -= value;
            } else {
                entry.0 += value;
            }
        }
    }

    let mut running = opening;
    buckets
        .into_iter()
        .map(|(period, (inflow, outflow))| {
            running += inflow - outflow;
            ProjectionPoint {
                period: period.format("%Y-%m").to_string(),
                inflow: inflow.to_string(),
                outflow: outflow.to_string(),
                closing: running.to_string(),
                projected: last_real.is_some_and(|last| period > month_start(last)),
            }
        })
        .collect()
}

/// The first projected date on which the matching accounts drop below
/// `threshold`, if any. Answers "when do I run out of money".
pub fn first_shortfall(
    all: &[ResolvedTransaction],
    selector: &AccountSelector<'_>,
    commodity: &str,
    price_db: &crate::price_db::PriceDb,
    threshold: rust_decimal::Decimal,
    after: Option<NaiveDate>,
) -> Option<ShortfallAlert> {
    use crate::reports::valued_quantity;
    use rust_decimal::Decimal;

    // Postings must be walked in date order for the running balance to mean
    // anything; forecast entries are appended, not merged, upstream.
    let mut lines: Vec<(NaiveDate, Decimal, &str)> = Vec::new();
    for txn in all {
        for posting in &txn.postings {
            if !selector.matches(&posting.account.full) {
                continue;
            }
            let (value, _unconvertible) =
                valued_quantity(&posting.amount, commodity, price_db, posting.date);
            if !value.is_zero() {
                lines.push((posting.date, value, txn.description.as_str()));
            }
        }
    }
    lines.sort_by_key(|(d, _, _)| *d);

    let mut running = Decimal::ZERO;
    for (date, value, description) in lines {
        running += value;
        if running < threshold && after.is_none_or(|a| date > a) {
            return Some(ShortfallAlert {
                date: date.format("%Y-%m-%d").to_string(),
                balance: running.to_string(),
                description: description.to_string(),
            });
        }
    }
    None
}

fn month_start(date: NaiveDate) -> NaiveDate {
    use chrono::Datelike;
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date)
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

#[cfg(test)]
mod projection_tests {
    use super::*;
    use crate::forecast;
    use crate::price_db::PriceDb;
    use hledger_parser::parse;
    use rust_decimal_macros::dec;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    const JOURNAL: &str = "\
~ monthly from 2024-02-01  Rent
    expenses:rent  $1000.00
    assets:checking

2024-01-31 Seed
    assets:checking  $2500.00
    equity:opening
";

    fn projected() -> (Vec<ResolvedTransaction>, NaiveDate) {
        let journal = parse(JOURNAL).unwrap();
        let real = resolve_transactions(&journal).unwrap();
        let last_real = real.iter().map(|t| t.date).max().unwrap();
        let all = with_forecast(&journal, &real, Some(d(2024, 5, 31)));
        (all, last_real)
    }

    #[test]
    fn projection_marks_future_periods_and_tracks_closing_balance() {
        let (all, last_real) = projected();
        let points = cash_flow_projection(
            &all,
            Some(last_real),
            &AccountSelector::Prefix("assets:checking"),
            "$",
            &PriceDb::default(),
            None,
            None,
        );

        // Jan is actual (+2500); Feb..May are projected rent of 1000 each.
        assert_eq!(points[0].period, "2024-01");
        assert!(!points[0].projected);
        assert_eq!(points[0].closing, "2500.00");

        assert!(points[1..].iter().all(|p| p.projected));
        assert_eq!(points[1].outflow, "1000.00");
        assert_eq!(points[1].closing, "1500.00");
        assert_eq!(points.last().unwrap().closing, "-1500.00");
    }

    #[test]
    fn shortfall_reports_the_first_date_the_balance_goes_negative() {
        let (all, _) = projected();
        let alert = first_shortfall(
            &all,
            &AccountSelector::Prefix("assets:checking"),
            "$",
            &PriceDb::default(),
            dec!(0),
            None,
        )
        .expect("balance should go negative");

        // 2500 covers Feb and Mar rent; the April payment overdraws it.
        assert_eq!(alert.date, "2024-04-01");
        assert_eq!(alert.balance, "-500.00");
        assert_eq!(alert.description, "Rent");
    }

    #[test]
    fn no_shortfall_when_balance_stays_above_threshold() {
        let journal = parse("2024-01-31 Seed\n    assets:checking  $10.00\n    equity:opening\n")
            .unwrap();
        let real = resolve_transactions(&journal).unwrap();
        assert!(first_shortfall(
            &real,
            &AccountSelector::Prefix("assets:checking"),
            "$",
            &PriceDb::default(),
            dec!(0),
            None
        )
        .is_none());
    }

    #[test]
    fn projection_starts_from_today_not_the_end_of_a_stale_journal() {
        let journal = parse(JOURNAL).unwrap();
        let real = resolve_transactions(&journal).unwrap();
        // Journal ends 2024-02-01 (last generated rent is not real); today is
        // far later. Replaying from the journal end would invent years of rent.
        let today = d(2026, 8, 21);
        let horizon = d(2027, 8, 21);
        let (start, end) = forecast::projection_window(&real, today, Some(horizon)).unwrap();
        assert_eq!(start, today);
        assert_eq!(end, horizon);

        let all = with_projection(&journal, &real, today, Some(horizon));
        let first_projected = all
            .iter()
            .filter(|t| t.postings.iter().any(|p| p.generated))
            .map(|t| t.date)
            .min()
            .unwrap();
        assert!(first_projected >= today, "projected into the past: {first_projected}");
    }

    #[test]
    fn projection_still_starts_after_a_journal_that_runs_into_the_future() {
        let journal = parse(
            "2027-01-01 Future\n    assets:checking  $10.00\n    equity:opening\n",
        )
        .unwrap();
        let real = resolve_transactions(&journal).unwrap();
        let today = d(2026, 8, 21);
        let (start, _) = forecast::projection_window(&real, today, Some(d(2028, 1, 1))).unwrap();
        assert_eq!(start, d(2027, 1, 2));
    }

    #[test]
    fn one_unbalanced_rule_does_not_silence_the_others() {
        // hledger hard-errors on an unbalanced rule under --forecast. We can't
        // refuse to open the file, so the good rule must still forecast and the
        // bad one must be reported rather than silently dropping everything.
        let journal = parse(
            concat!(
                "~ monthly from 2024-02-01  Good\n",
                "    expenses:rent  $100.00\n",
                "    assets:cash\n\n",
                "~ monthly from 2024-02-01  Bad\n",
                "    expenses:a  $10.00\n",
                "    expenses:b  $20.00\n\n",
                "2024-01-05 Seed\n",
                "    assets:cash  $500.00\n",
                "    equity:opening\n",
            ),
        )
        .unwrap();

        let outcome = forecast::forecast_checked(&journal, d(2024, 2, 1), d(2024, 4, 30));
        assert_eq!(outcome.transactions.len(), 3, "the good rule must still run");
        assert!(outcome.transactions.iter().all(|t| t.description == "Good"));
        assert_eq!(outcome.errors.len(), 1);
        assert!(
            outcome.errors[0].1.contains("balance"),
            "unhelpful error: {}",
            outcome.errors[0].1
        );
    }

    #[test]
    fn rules_are_extracted_with_unparseable_ones_flagged() {
        let journal = parse(
            "~ monthly  Good\n    expenses:a  $1.00\n    assets:cash\n\n~ fortnightly  Bad\n    expenses:b  $2.00\n    assets:cash\n",
        )
        .unwrap();
        let rules = extract_rules(&journal);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].description, "Good");
        assert!(rules[0].error.is_none());
        // The elided balancing posting is preserved as an amount-less entry.
        assert_eq!(rules[0].postings[1].account, "assets:cash");
        assert!(rules[0].postings[1].amount.is_none());
        assert!(rules[1].error.is_some());
    }
}
