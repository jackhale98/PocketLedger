//! Multi-period columnar balance reports: the engine behind hledger's
//! `bal -W/-M/-Q/-Y [--depth N] [--historical|--cumulative]`.

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::amount::MixedAmount;
use crate::balance::ResolvedTransaction;
use crate::query::Query;
use crate::reports::AmountEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportInterval {
    Weekly,
    Monthly,
    Quarterly,
    Yearly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccumulationMode {
    /// Change during each period (hledger default).
    Periodic,
    /// Running total from the report start.
    Cumulative,
    /// Running total from the beginning of the journal (bal -H).
    Historical,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodicBalanceRow {
    pub account: String,
    pub depth: usize,
    /// One cell per period, in period order.
    pub amounts: Vec<Vec<AmountEntry>>,
    /// Row total (periodic/cumulative: sum of changes; historical: final balance).
    pub total: Vec<AmountEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodicBalanceReport {
    /// Period labels, e.g. "2024-01" / "2024-Q1" / "2024" / "2024-W05".
    pub periods: Vec<String>,
    pub rows: Vec<PeriodicBalanceRow>,
    /// Column totals across all rows.
    pub totals: Vec<Vec<AmountEntry>>,
}

/// Convert what has a price, keep the rest in its own units — a holding with
/// no price must not vanish from a column.
fn value_at(
    m: &MixedAmount,
    target: &str,
    price_db: &crate::price_db::PriceDb,
    date: NaiveDate,
) -> MixedAmount {
    if target.is_empty() {
        return m.clone();
    }
    crate::reports::convert_mixed(m, target, price_db, date)
}

fn mixed_entries(m: &MixedAmount) -> Vec<AmountEntry> {
    if m.amounts.is_empty() {
        return vec![];
    }
    m.amounts
        .iter()
        .map(|(c, q)| AmountEntry {
            commodity: c.clone(),
            quantity: q.normalize().to_string(),
        })
        .collect()
}

fn period_start(date: NaiveDate, interval: ReportInterval) -> NaiveDate {
    match interval {
        ReportInterval::Weekly => {
            date - Duration::days(date.weekday().num_days_from_monday() as i64)
        }
        ReportInterval::Monthly => NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap(),
        ReportInterval::Quarterly => {
            let q_month = ((date.month() - 1) / 3) * 3 + 1;
            NaiveDate::from_ymd_opt(date.year(), q_month, 1).unwrap()
        }
        ReportInterval::Yearly => NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap(),
    }
}

fn next_period(start: NaiveDate, interval: ReportInterval) -> NaiveDate {
    match interval {
        ReportInterval::Weekly => start + Duration::weeks(1),
        ReportInterval::Monthly => {
            if start.month() == 12 {
                NaiveDate::from_ymd_opt(start.year() + 1, 1, 1).unwrap()
            } else {
                NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1).unwrap()
            }
        }
        ReportInterval::Quarterly => {
            let m = start.month();
            if m >= 10 {
                NaiveDate::from_ymd_opt(start.year() + 1, 1, 1).unwrap()
            } else {
                NaiveDate::from_ymd_opt(start.year(), m + 3, 1).unwrap()
            }
        }
        ReportInterval::Yearly => NaiveDate::from_ymd_opt(start.year() + 1, 1, 1).unwrap(),
    }
}

fn period_label(start: NaiveDate, interval: ReportInterval) -> String {
    match interval {
        ReportInterval::Weekly => format!("{}-W{:02}", start.iso_week().year(), start.iso_week().week()),
        ReportInterval::Monthly => start.format("%Y-%m").to_string(),
        ReportInterval::Quarterly => {
            format!("{}-Q{}", start.year(), (start.month() - 1) / 3 + 1)
        }
        ReportInterval::Yearly => start.year().to_string(),
    }
}

/// Clip an account name to a depth (1 = top level). Depth 0/None = no clip.
fn clip_account(account: &str, depth: Option<usize>) -> String {
    match depth {
        Some(d) if d > 0 => account
            .split(':')
            .take(d)
            .collect::<Vec<_>>()
            .join(":"),
        _ => account.to_string(),
    }
}

/// Build the multi-period balance report.
///
/// - `interval` buckets postings by posting date.
/// - `mode` controls accumulation (periodic change / cumulative / historical).
/// - `depth` aggregates deeper accounts into their depth-N ancestor, so rows
///   are disjoint and column totals are simple sums.
/// - `query` filters postings (its own `depth:` term applies when `depth` is
///   None).
pub fn periodic_balance_report(
    transactions: &[ResolvedTransaction],
    interval: ReportInterval,
    mode: AccumulationMode,
    depth: Option<usize>,
    query: Option<&Query>,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    // `target` empty leaves each commodity as it is.
    target: &str,
    price_db: &crate::price_db::PriceDb,
) -> PeriodicBalanceReport {
    let depth = depth.or_else(|| query.and_then(|q| q.depth()));

    // Collect matching postings.
    struct Line {
        date: NaiveDate,
        account: String,
        amount: MixedAmount,
    }
    let mut lines: Vec<Line> = Vec::new();
    for txn in transactions {
        for posting in &txn.postings {
            if let Some(q) = query {
                if !q.matches_posting(txn, posting) {
                    continue;
                }
            }
            lines.push(Line {
                date: posting.date,
                account: clip_account(&posting.account.full, depth),
                amount: posting.amount.clone(),
            });
        }
    }

    if lines.is_empty() {
        return PeriodicBalanceReport {
            periods: vec![],
            rows: vec![],
            totals: vec![],
        };
    }

    let min_date = lines.iter().map(|l| l.date).min().unwrap();
    let max_date = lines.iter().map(|l| l.date).max().unwrap();

    let report_from = date_from.unwrap_or(min_date);
    let report_to = date_to.unwrap_or(max_date);

    // Period boundaries.
    let mut period_starts = Vec::new();
    let mut cursor = period_start(report_from, interval);
    while cursor <= report_to {
        period_starts.push(cursor);
        cursor = next_period(cursor, interval);
    }
    if period_starts.is_empty() {
        period_starts.push(period_start(report_from, interval));
    }

    let period_index = |date: NaiveDate| -> Option<usize> {
        if date > report_to {
            return None;
        }
        let start = period_start(date, interval);
        period_starts.iter().position(|p| *p == start)
    };

    // Per-account, per-period changes. Postings before the report window
    // contribute to the "opening" bucket used by historical mode.
    let mut changes: BTreeMap<String, Vec<MixedAmount>> = BTreeMap::new();
    let mut opening: BTreeMap<String, MixedAmount> = BTreeMap::new();
    let n = period_starts.len();
    // Prices are looked up at each column's own end date; the last column is
    // capped at the report end so a partial period isn't valued in the future.
    let period_ends: Vec<NaiveDate> = period_starts
        .iter()
        .map(|start| {
            next_period(*start, interval)
                .pred_opt()
                .unwrap_or(*start)
                .min(report_to)
        })
        .collect();

    for line in &lines {
        if line.date < report_from {
            opening
                .entry(line.account.clone())
                .or_insert_with(MixedAmount::zero)
                .add_mixed(&line.amount);
            continue;
        }
        let Some(idx) = period_index(line.date) else {
            continue;
        };
        let row = changes
            .entry(line.account.clone())
            .or_insert_with(|| vec![MixedAmount::zero(); n]);
        row[idx].add_mixed(&line.amount);
    }

    // Historical mode also needs rows for accounts with only pre-window
    // activity (their opening balance is their whole story).
    if mode == AccumulationMode::Historical {
        for account in opening.keys() {
            changes
                .entry(account.clone())
                .or_insert_with(|| vec![MixedAmount::zero(); n]);
        }
    }

    // Build rows in the requested accumulation mode.
    let mut rows = Vec::new();
    let mut column_totals = vec![MixedAmount::zero(); n];

    for (account, per_period) in &changes {
        let mut cells: Vec<MixedAmount> = Vec::with_capacity(n);
        let mut running = match mode {
            AccumulationMode::Historical => {
                opening.get(account).cloned().unwrap_or_default()
            }
            _ => MixedAmount::zero(),
        };

        for change in per_period {
            match mode {
                AccumulationMode::Periodic => cells.push(change.clone()),
                AccumulationMode::Cumulative | AccumulationMode::Historical => {
                    running.add_mixed(change);
                    cells.push(running.clone());
                }
            }
        }

        // Row total: periodic = sum of changes; cumulative/historical = the
        // final balance (summing balances would be meaningless).
        let total = match mode {
            AccumulationMode::Periodic => {
                let mut t = MixedAmount::zero();
                for c in &cells {
                    t.add_mixed(c);
                }
                t
            }
            _ => cells.last().cloned().unwrap_or_default(),
        };

        // Skip all-zero rows (parity with hledger's default).
        if cells.iter().all(|c| c.is_zero()) && total.is_zero() {
            continue;
        }

        for (i, c) in cells.iter().enumerate() {
            match mode {
                AccumulationMode::Periodic => column_totals[i].add_mixed(c),
                // For balance modes the column total is the sum of balances.
                _ => column_totals[i].add_mixed(c),
            }
        }

        rows.push(PeriodicBalanceRow {
            account: account.clone(),
            depth: account.matches(':').count() + 1,
            amounts: cells
                .iter()
                .enumerate()
                .map(|(i, c)| mixed_entries(&value_at(c, target, price_db, period_ends[i])))
                .collect(),
            total: mixed_entries(&value_at(&total, target, price_db, report_to)),
        });
    }

    PeriodicBalanceReport {
        periods: period_starts
            .iter()
            .map(|p| period_label(*p, interval))
            .collect(),
        rows,
        totals: column_totals
            .iter()
            .enumerate()
            .map(|(i, c)| mixed_entries(&value_at(c, target, price_db, period_ends[i])))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::balance::resolve_transactions;
    use crate::query::parse_query;
    use hledger_parser::parse;

    fn txns() -> Vec<ResolvedTransaction> {
        let input = "\
2024-01-05 A
    expenses:food:coffee  $10.00
    assets:checking

2024-01-20 B
    expenses:food:grocery  $40.00
    assets:checking

2024-02-10 C
    expenses:food:coffee  $5.00
    assets:checking

2024-03-01 D
    expenses:rent  $1200.00
    assets:checking
";
        resolve_transactions(&parse(input).unwrap()).unwrap()
    }

    fn cell(row: &PeriodicBalanceRow, i: usize) -> String {
        row.amounts[i]
            .iter()
            .map(|e| format!("{}{}", e.commodity, e.quantity))
            .collect::<Vec<_>>()
            .join(",")
    }

    #[test]
    fn monthly_periodic_changes() {
        let t = txns();
        let r = periodic_balance_report(
            &t,
            ReportInterval::Monthly,
            AccumulationMode::Periodic,
            None,
            None,
            None,
            None,
            "",
            &crate::price_db::PriceDb::default(),
        );
        assert_eq!(r.periods, vec!["2024-01", "2024-02", "2024-03"]);

        let coffee = r.rows.iter().find(|x| x.account == "expenses:food:coffee").unwrap();
        assert_eq!(cell(coffee, 0), "$10");
        assert_eq!(cell(coffee, 1), "$5");
        assert_eq!(cell(coffee, 2), "");
        assert_eq!(coffee.total[0].quantity, "15");
    }

    #[test]
    fn depth_clipping_aggregates() {
        let t = txns();
        let r = periodic_balance_report(
            &t,
            ReportInterval::Monthly,
            AccumulationMode::Periodic,
            Some(2),
            None,
            None,
            None,
            "",
            &crate::price_db::PriceDb::default(),
        );
        let food = r.rows.iter().find(|x| x.account == "expenses:food").unwrap();
        assert_eq!(cell(food, 0), "$50");
        assert_eq!(cell(food, 1), "$5");
        assert!(r.rows.iter().all(|x| x.account.split(':').count() <= 2));
    }

    #[test]
    fn cumulative_mode_accumulates_from_report_start() {
        let t = txns();
        let r = periodic_balance_report(
            &t,
            ReportInterval::Monthly,
            AccumulationMode::Cumulative,
            Some(1),
            None,
            None,
            None,
            "",
            &crate::price_db::PriceDb::default(),
        );
        let exp = r.rows.iter().find(|x| x.account == "expenses").unwrap();
        assert_eq!(cell(exp, 0), "$50");
        assert_eq!(cell(exp, 1), "$55");
        assert_eq!(cell(exp, 2), "$1255");
        assert_eq!(exp.total[0].quantity, "1255");
    }

    #[test]
    fn historical_mode_includes_opening_balances() {
        let t = txns();
        let r = periodic_balance_report(
            &t,
            ReportInterval::Monthly,
            AccumulationMode::Historical,
            Some(1),
            None,
            Some(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()),
            None,
            "",
            &crate::price_db::PriceDb::default(),
        );
        assert_eq!(r.periods, vec!["2024-02", "2024-03"]);
        let exp = r.rows.iter().find(|x| x.account == "expenses").unwrap();
        // Feb balance includes January's $50 opening.
        assert_eq!(cell(exp, 0), "$55");
        assert_eq!(cell(exp, 1), "$1255");
    }

    #[test]
    fn quarterly_and_yearly_labels() {
        let t = txns();
        let q = periodic_balance_report(
            &t,
            ReportInterval::Quarterly,
            AccumulationMode::Periodic,
            Some(1),
            None,
            None,
            None,
            "",
            &crate::price_db::PriceDb::default(),
        );
        assert_eq!(q.periods, vec!["2024-Q1"]);
        let y = periodic_balance_report(
            &t,
            ReportInterval::Yearly,
            AccumulationMode::Periodic,
            Some(1),
            None,
            None,
            None,
            "",
            &crate::price_db::PriceDb::default(),
        );
        assert_eq!(y.periods, vec!["2024"]);
    }

    #[test]
    fn query_filters_rows() {
        let t = txns();
        let query = parse_query("acct:expenses not:rent").unwrap();
        let r = periodic_balance_report(
            &t,
            ReportInterval::Monthly,
            AccumulationMode::Periodic,
            Some(2),
            Some(&query),
            None,
            None,
            "",
            &crate::price_db::PriceDb::default(),
        );
        assert!(r.rows.iter().any(|x| x.account == "expenses:food"));
        assert!(!r.rows.iter().any(|x| x.account == "expenses:rent"));
        assert!(!r.rows.iter().any(|x| x.account.starts_with("assets")));
    }

    #[test]
    fn column_totals_sum_rows() {
        let t = txns();
        let r = periodic_balance_report(
            &t,
            ReportInterval::Monthly,
            AccumulationMode::Periodic,
            Some(1),
            None,
            None,
            None,
            "",
            &crate::price_db::PriceDb::default(),
        );
        // Balanced journal: every column total is zero (expenses + assets).
        for total in &r.totals {
            assert!(total.is_empty(), "expected zero column total, got {:?}", total);
        }
    }
}
