//! Investment returns: money-weighted (IRR) and time-weighted (TWR).
//!
//! Mirrors `hledger roi`. The two answer different questions and routinely
//! disagree, which is the point of showing both:
//!
//! * **IRR** is the rate that makes the investor's own cash flows balance, so
//!   it reflects *when* money was put in. Contributing just before a good run
//!   flatters it.
//! * **TWR** chains the return of each period between cash flows, removing the
//!   effect of contribution timing. It is what you compare against an index,
//!   because the index had no contributions.
//!
//! A cash flow is money crossing the boundary of the investment from outside.
//! Growth inside it — dividends, appreciation, whatever the profit-and-loss
//! query selects — is return, not a cash flow, and counting it as one would
//! make every gain look like a deposit and report a return of zero.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use crate::balance::ResolvedTransaction;
use crate::price_db::PriceDb;
use crate::query::Query;
use crate::reports::valued_quantity;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CashFlow {
    pub date: String,
    /// Positive when money enters the investment.
    pub amount: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoiReport {
    pub begin: String,
    pub end: String,
    pub value_begin: String,
    pub value_end: String,
    /// Net money added over the period.
    pub cashflow: String,
    /// Value gained beyond what was contributed.
    pub pnl: String,
    /// Money-weighted return, annualised. None when it cannot be solved —
    /// with no sign change in the flows there is no rate that balances them.
    pub irr: Option<String>,
    /// Time-weighted return over the whole period, and annualised.
    pub twr_period: Option<String>,
    pub twr_annual: Option<String>,
    pub commodity: String,
    /// Commodities the investment accounts actually hold at the end, in their
    /// own units. Holding something other than `commodity` is what makes an
    /// account an investment rather than a place money sits.
    pub held_commodities: Vec<String>,
    pub flows: Vec<CashFlow>,
}

/// Value held by the investment accounts on `date`.
fn value_on(
    transactions: &[ResolvedTransaction],
    investment: &Query,
    date: NaiveDate,
    commodity: &str,
    price_db: &PriceDb,
) -> Decimal {
    let mut total = Decimal::ZERO;
    for txn in transactions {
        for posting in &txn.postings {
            if posting.date > date || !investment.matches_posting(txn, posting) {
                continue;
            }
            let (value, _) = valued_quantity(&posting.amount, commodity, price_db, date);
            total += value;
        }
    }
    total
}

/// Commodities with a non-zero balance in the investment accounts on `date`.
fn held_commodities(
    transactions: &[ResolvedTransaction],
    investment: &Query,
    date: NaiveDate,
) -> Vec<String> {
    let mut totals: BTreeMap<String, Decimal> = BTreeMap::new();
    for txn in transactions {
        for posting in &txn.postings {
            if posting.date > date || !investment.matches_posting(txn, posting) {
                continue;
            }
            for (commodity, qty) in &posting.amount.amounts {
                *totals.entry(commodity.clone()).or_default() += *qty;
            }
        }
    }
    totals
        .into_iter()
        .filter(|(_, qty)| !qty.is_zero())
        .map(|(commodity, _)| commodity)
        .collect()
}

/// Money crossing into the investment, by date.
///
/// A transaction that touches the investment moves money in from outside
/// exactly when it also touches an account that is neither the investment nor
/// profit-and-loss. The amount is what those outside accounts gave up.
fn cash_flows(
    transactions: &[ResolvedTransaction],
    investment: &Query,
    pnl: Option<&Query>,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<(NaiveDate, Decimal)> {
    let mut flows: Vec<(NaiveDate, Decimal)> = Vec::new();

    for txn in transactions {
        let touches_investment = txn
            .postings
            .iter()
            .any(|p| investment.matches_posting(txn, p));
        if !touches_investment {
            continue;
        }

        let mut outside = Decimal::ZERO;
        let mut date = txn.date;
        for posting in &txn.postings {
            if investment.matches_posting(txn, posting) {
                date = posting.date;
                continue;
            }
            if pnl.is_some_and(|q| q.matches_posting(txn, posting)) {
                continue;
            }
            for (_, qty) in &posting.amount.amounts {
                outside += *qty;
            }
        }

        if outside.is_zero() || date < from || date > to {
            continue;
        }
        // The outside account gave the money up, so it entered as its negation.
        flows.push((date, -outside));
    }

    flows.sort_by_key(|(d, _)| *d);
    flows
}

fn year_fraction(from: NaiveDate, to: NaiveDate) -> f64 {
    (to - from).num_days() as f64 / 365.0
}

/// Net present value of the flows at rate `r`, with the terminal value as a
/// final inflow.
fn npv(flows: &[(NaiveDate, f64)], rate: f64) -> f64 {
    let Some((start, _)) = flows.first() else {
        return 0.0;
    };
    flows
        .iter()
        .map(|(date, amount)| amount / (1.0 + rate).powf(year_fraction(*start, *date)))
        .sum()
}

/// Solve for the rate where NPV is zero.
///
/// Bisection rather than Newton: the NPV curve is well behaved between the
/// bracketing rates but its derivative is not, and a solver that fails to
/// converge silently is worse here than one that reports it cannot.
fn solve_irr(flows: &[(NaiveDate, f64)]) -> Option<f64> {
    if flows.len() < 2 {
        return None;
    }
    // Without both an outflow and an inflow no rate balances the series.
    if !flows.iter().any(|(_, a)| *a > 0.0) || !flows.iter().any(|(_, a)| *a < 0.0) {
        return None;
    }

    let (mut low, mut high) = (-0.9999, 100.0);
    let (mut f_low, f_high) = (npv(flows, low), npv(flows, high));
    if f_low * f_high > 0.0 {
        return None;
    }

    for _ in 0..200 {
        let mid = (low + high) / 2.0;
        let f_mid = npv(flows, mid);
        if f_mid.abs() < 1e-9 {
            return Some(mid);
        }
        if f_low * f_mid < 0.0 {
            high = mid;
        } else {
            low = mid;
            f_low = f_mid;
        }
    }
    Some((low + high) / 2.0)
}

/// Chain the return of each span between cash flows.
///
/// A span ends immediately *before* a flow, so the growth measured is the
/// holding's own, not the jump caused by paying money in. The last span runs
/// to the exclusive end of the period, which is what picks up gains dated on
/// the final day.
fn time_weighted(
    transactions: &[ResolvedTransaction],
    investment: &Query,
    flows: &[(NaiveDate, Decimal)],
    from: NaiveDate,
    to: NaiveDate,
    commodity: &str,
    price_db: &PriceDb,
) -> Option<f64> {
    let end_exclusive = to.succ_opt().unwrap_or(to);

    let mut boundaries: Vec<NaiveDate> = flows.iter().map(|(d, _)| *d).collect();
    boundaries.push(end_exclusive);
    boundaries.dedup();

    let mut growth = 1.0f64;
    let mut start_value = value_on(
        transactions,
        investment,
        from.pred_opt().unwrap_or(from),
        commodity,
        price_db,
    );

    for boundary in boundaries {
        let value_at = value_on(transactions, investment, boundary, commodity, price_db);
        let flow_here: Decimal = flows
            .iter()
            .filter(|(d, _)| *d == boundary)
            .map(|(_, a)| *a)
            .sum();
        let before_flow = value_at - flow_here;

        if start_value > Decimal::ZERO {
            let rate = (before_flow / start_value).to_f64()? - 1.0;
            growth *= 1.0 + rate;
        }
        start_value = value_at;
    }

    Some(growth - 1.0)
}

pub fn roi(
    transactions: &[ResolvedTransaction],
    investment: &Query,
    pnl: Option<&Query>,
    from: NaiveDate,
    to: NaiveDate,
    commodity: &str,
    price_db: &PriceDb,
) -> RoiReport {
    let value_begin = value_on(
        transactions,
        investment,
        from.pred_opt().unwrap_or(from),
        commodity,
        price_db,
    );
    let value_end = value_on(transactions, investment, to, commodity, price_db);
    let flows = cash_flows(transactions, investment, pnl, from, to);
    let contributed: Decimal = flows.iter().map(|(_, a)| *a).sum();
    let pnl_amount = value_end - value_begin - contributed;

    // For IRR the investor's contributions are outflows and the closing value
    // is a final inflow.
    let mut irr_flows: Vec<(NaiveDate, f64)> = Vec::new();
    if !value_begin.is_zero() {
        irr_flows.push((from, -value_begin.to_f64().unwrap_or(0.0)));
    }
    for (date, amount) in &flows {
        irr_flows.push((*date, -amount.to_f64().unwrap_or(0.0)));
    }
    irr_flows.push((
        to.succ_opt().unwrap_or(to),
        value_end.to_f64().unwrap_or(0.0),
    ));

    let irr = solve_irr(&irr_flows);
    let twr_period = time_weighted(
        transactions, investment, &flows, from, to, commodity, price_db,
    );
    let years = year_fraction(from, to.succ_opt().unwrap_or(to)).max(1e-9);
    let twr_annual = twr_period.map(|r| (1.0 + r).powf(1.0 / years) - 1.0);

    let pct = |r: f64| format!("{:.2}", r * 100.0);

    RoiReport {
        begin: from.to_string(),
        end: to.to_string(),
        value_begin: value_begin.round_dp(2).to_string(),
        value_end: value_end.round_dp(2).to_string(),
        cashflow: contributed.round_dp(2).to_string(),
        pnl: pnl_amount.round_dp(2).to_string(),
        irr: irr.map(pct),
        twr_period: twr_period.map(pct),
        twr_annual: twr_annual.map(pct),
        commodity: commodity.to_string(),
        held_commodities: held_commodities(transactions, investment, to),
        flows: flows
            .iter()
            .map(|(date, amount)| CashFlow {
                date: date.to_string(),
                amount: amount.round_dp(2).to_string(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parse_query;

    fn report(text: &str, investment: &str, pnl: &str) -> RoiReport {
        let journal = hledger_parser::parse(text).unwrap();
        let txns = crate::balance::resolve_transactions(&journal).unwrap();
        roi(
            &txns,
            &parse_query(investment).unwrap(),
            Some(&parse_query(pnl).unwrap()),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            "$",
            &crate::price_db::PriceDb::default(),
        )
    }

    /// The default profit query the UI sends. Kept here so the two can't drift
    /// apart silently.
    const PNL_DEFAULT: &str = "acct:interest|dividend|gain|loss|capital|realized|unrealized|distribution|yield|commission|brokerage";

    /// A salary is not a return on a current account. With the default profit
    /// query a plain cash account must report no gain, which is what keeps the
    /// returns panel off accounts that have none.
    #[test]
    fn salary_into_checking_is_not_a_return() {
        let r = report(
            concat!(
                "2024-01-05 groceries\n    expenses:food      $50.00\n    assets:checking\n\n",
                "2024-02-05 paycheck\n    assets:checking  $2000.00\n    income:salary\n",
            ),
            "acct:assets:checking",
            PNL_DEFAULT,
        );
        assert_eq!(r.pnl, "0.00");
        assert_eq!(r.held_commodities, vec!["$".to_string()]);
        // No gain, so whatever the solver returns must be a flat return.
        let irr: f64 = r.irr.as_deref().unwrap_or("0").parse().unwrap();
        assert!(irr.abs() < 0.01, "flat account returned {irr}%");
    }

    /// Interest on a savings account is a return, and is what makes the panel
    /// appear for an account holding nothing but the base currency.
    #[test]
    fn interest_counts_as_return() {
        let r = report(
            concat!(
                "2024-01-01 open\n    assets:savings   $1000.00\n    assets:checking\n\n",
                "2024-07-01 interest\n    assets:savings     $20.00\n    income:interest\n",
            ),
            "acct:assets:savings",
            PNL_DEFAULT,
        );
        assert_eq!(r.pnl, "20.00");
        assert!(r.irr.is_some());
    }

    /// Holding units of something other than the valuation currency is the
    /// other signal the panel keys off.
    #[test]
    fn shares_are_reported_as_held_units() {
        let r = report(
            concat!(
                "2024-01-01 buy\n    assets:broker    10 VTSAX @ $100.00\n    assets:checking\n",
            ),
            "acct:assets:broker",
            PNL_DEFAULT,
        );
        assert_eq!(r.held_commodities, vec!["VTSAX".to_string()]);
    }
}
