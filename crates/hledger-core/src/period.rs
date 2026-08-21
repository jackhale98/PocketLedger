//! Period expressions (`~ monthly`, `every 2 weeks from 2024-01`, …) and the
//! occurrence dates they generate.
//!
//! Shared by budgets (which count occurrences to scale goals) and forecasts
//! (which materialize an actual transaction per occurrence), because hledger
//! drives both from the same periodic transaction rules.
//!
//! Anchoring follows hledger: a rule with an explicit `from DATE` keeps its
//! own grid, while a bare rule like `~ monthly` is anchored to the START OF
//! THE REPORTING WINDOW, not to the calendar grid. `hledger print --forecast`
//! over a window starting 2024-03-15 puts a `~ monthly` rule on the 15th of
//! each month, not the 1st.

use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};
use serde::Serialize;

/// Guard against a malformed spec producing an unbounded occurrence loop.
const MAX_STEPS: u32 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PeriodUnit {
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

/// A parsed hledger period expression: `[every N] UNIT [from DATE] [to DATE]`.
/// Unrecognized expressions are rejected with a warning — silently defaulting
/// to monthly falsified budgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodSpec {
    pub unit: PeriodUnit,
    pub every: u32,
    pub start: Option<NaiveDate>,
    /// Exclusive end bound (`to DATE` in hledger).
    pub end: Option<NaiveDate>,
    /// `every Nth day of month` — occurrences land on this day of the month.
    pub day_of_month: Option<u32>,
    /// `every <weekday>` — occurrences land on this weekday.
    #[serde(skip)]
    pub weekday: Option<Weekday>,
    /// The original period expression text.
    pub raw: String,
}

impl PeriodSpec {
    /// A plain interval with no explicit start, day-of-month or weekday
    /// anchor — the case hledger anchors to the reporting window start.
    pub fn is_floating(&self) -> bool {
        self.start.is_none() && self.day_of_month.is_none() && self.weekday.is_none()
    }
}

/// Parse a period expression. Returns None (→ warning upstream) for syntax we
/// can't faithfully honor.
pub fn parse_period_expression(s: &str) -> Option<PeriodSpec> {
    let raw = s.trim().to_string();
    let lower = raw.to_lowercase();
    let mut tokens = lower.split_whitespace().peekable();

    let mut unit: Option<PeriodUnit> = None;
    let mut every: u32 = 1;
    let mut start: Option<NaiveDate> = None;
    let mut end: Option<NaiveDate> = None;
    let mut day_of_month: Option<u32> = None;
    let mut weekday: Option<Weekday> = None;

    while let Some(tok) = tokens.next() {
        match tok {
            "daily" | "every day" => unit = Some(PeriodUnit::Day),
            "weekly" => unit = Some(PeriodUnit::Week),
            "monthly" => unit = Some(PeriodUnit::Month),
            "quarterly" => unit = Some(PeriodUnit::Quarter),
            "yearly" | "annually" => unit = Some(PeriodUnit::Year),
            "every" => {
                let next = tokens.next()?;

                // "every 15th day of month" / "every 2nd monday of month"
                if let Some(nth) = parse_ordinal(next) {
                    // The word after the ordinal says what recurs.
                    let what = tokens.next()?;
                    if let Some(w) = parse_weekday(what) {
                        // "every 2nd monday of month" — we can't place an
                        // Nth-weekday-of-month faithfully, so reject rather
                        // than generate wrong dates.
                        let _ = w;
                        return None;
                    }
                    if what != "day" {
                        return None;
                    }
                    // Optional trailing "of month" / "of week".
                    if tokens.peek() == Some(&"of") {
                        tokens.next();
                        match tokens.next()? {
                            "month" => {}
                            _ => return None,
                        }
                    }
                    if !(1..=31).contains(&nth) {
                        return None;
                    }
                    day_of_month = Some(nth);
                    unit = Some(PeriodUnit::Month);
                    continue;
                }

                // "every monday"
                if let Some(w) = parse_weekday(next) {
                    weekday = Some(w);
                    unit = Some(PeriodUnit::Week);
                    continue;
                }

                // "every 2 weeks" or "every week"
                if let Ok(n) = next.parse::<u32>() {
                    if n == 0 {
                        return None;
                    }
                    every = n;
                    let u = tokens.next()?;
                    unit = Some(parse_unit_word(u)?);
                } else {
                    unit = Some(parse_unit_word(next)?);
                }
            }
            "from" => {
                let d = tokens.next()?;
                start = Some(parse_smart_date(d)?);
            }
            "to" | "until" => {
                let d = tokens.next()?;
                end = Some(parse_smart_date(d)?);
            }
            "in" => {
                // "in 2026-03": a single month range
                let d = tokens.next()?;
                start = Some(parse_smart_date(d)?);
                end = Some(smart_date_range_end(d)?);
            }
            _ => {
                // A bare date range "2026-01..2026-06" or unknown syntax.
                if let Some((a, b)) = tok.split_once("..") {
                    start = Some(parse_smart_date(a)?);
                    end = Some(parse_smart_date(b)?);
                } else {
                    return None;
                }
            }
        }
    }

    let unit = match unit {
        Some(u) => u,
        // Pure date range with no interval: treat as a single monthly-style
        // occurrence at the range start; reject when there is no range at all.
        None if start.is_some() => PeriodUnit::Month,
        None => return None,
    };

    Some(PeriodSpec {
        unit,
        every,
        start,
        end,
        day_of_month,
        weekday,
        raw,
    })
}

/// "15th" / "1st" / "2nd" / "3rd" / "22nd" → the number.
fn parse_ordinal(w: &str) -> Option<u32> {
    let trimmed = w.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    if trimmed.is_empty() || trimmed.len() == w.len() {
        return None;
    }
    let suffix = &w[trimmed.len()..];
    match suffix {
        "st" | "nd" | "rd" | "th" => trimmed.parse().ok(),
        _ => None,
    }
}

fn parse_weekday(w: &str) -> Option<Weekday> {
    match w.trim_end_matches(&[',', 's'][..]) {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" | "tues" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" | "thur" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

fn parse_unit_word(w: &str) -> Option<PeriodUnit> {
    match w.trim_end_matches(',') {
        "day" | "days" => Some(PeriodUnit::Day),
        "week" | "weeks" => Some(PeriodUnit::Week),
        "month" | "months" => Some(PeriodUnit::Month),
        "quarter" | "quarters" => Some(PeriodUnit::Quarter),
        "year" | "years" => Some(PeriodUnit::Year),
        _ => None,
    }
}

/// Parse YYYY, YYYY-MM, or YYYY-MM-DD to the first day of the named period.
///
/// End bounds need no adjustment: hledger reads `to 2024-05` and `..2024-05`
/// alike as "up to but not including 2024-05-01" (verified against hledger
/// 1.50.3). Only `in 2024-05`, which names a whole period, extends to the
/// following month — see [`smart_date_range_end`].
pub fn parse_smart_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(['-', '/', '.']).collect();
    match parts.len() {
        1 => {
            let y: i32 = parts[0].parse().ok()?;
            if !(1000..10000).contains(&y) {
                return None;
            }
            NaiveDate::from_ymd_opt(y, 1, 1)
        }
        2 => {
            let y: i32 = parts[0].parse().ok()?;
            let m: u32 = parts[1].parse().ok()?;
            NaiveDate::from_ymd_opt(y, m, 1)
        }
        3 => {
            let y: i32 = parts[0].parse().ok()?;
            let m: u32 = parts[1].parse().ok()?;
            let d: u32 = parts[2].parse().ok()?;
            NaiveDate::from_ymd_opt(y, m, d)
        }
        _ => None,
    }
}

fn smart_date_range_end(s: &str) -> Option<NaiveDate> {
    let start = parse_smart_date(s)?;
    let parts = s.split(['-', '/', '.']).count();
    Some(match parts {
        1 => NaiveDate::from_ymd_opt(start.year() + 1, 1, 1).unwrap_or(start),
        2 => add_months(start, 1),
        _ => start.succ_opt().unwrap_or(start),
    })
}

fn add_months(d: NaiveDate, n: u32) -> NaiveDate {
    d.checked_add_months(Months::new(n)).unwrap_or(d)
}

/// Snap a date back to the start of its containing period.
pub fn align_to_unit(date: NaiveDate, unit: PeriodUnit) -> NaiveDate {
    match unit {
        PeriodUnit::Day => date,
        PeriodUnit::Week => date - Duration::days(date.weekday().num_days_from_monday() as i64),
        PeriodUnit::Month => NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date),
        PeriodUnit::Quarter => {
            let q_month = ((date.month() - 1) / 3) * 3 + 1;
            NaiveDate::from_ymd_opt(date.year(), q_month, 1).unwrap_or(date)
        }
        PeriodUnit::Year => NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap_or(date),
    }
}

/// Advance one interval.
pub fn step(date: NaiveDate, unit: PeriodUnit, every: u32) -> Option<NaiveDate> {
    match unit {
        PeriodUnit::Day => date.checked_add_signed(Duration::days(every as i64)),
        PeriodUnit::Week => date.checked_add_signed(Duration::weeks(every as i64)),
        PeriodUnit::Month => date.checked_add_months(Months::new(every)),
        PeriodUnit::Quarter => date.checked_add_months(Months::new(3 * every)),
        PeriodUnit::Year => date.checked_add_months(Months::new(12 * every)),
    }
}

/// Where an unanchored rule (`~ monthly`, no `from`) places its occurrences.
///
/// The two callers genuinely differ, and conflating them produces wrong
/// numbers in one of them:
///
/// * [`Anchoring::Grid`] — budgets. A `~ monthly` goal recurs on the 1st, so
///   a mid-month reporting range touches only the occurrences that actually
///   fall inside it. Counting calendar-touched periods instead would inflate
///   goals for partial ranges.
/// * [`Anchoring::WindowStart`] — forecasts, matching hledger. `hledger print
///   --forecast=2024-03-15..2024-07-01` puts a `~ monthly` rule on the 15th of
///   each month, because the rule inherits the forecast window's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchoring {
    Grid,
    WindowStart,
}

fn anchor_for(spec: &PeriodSpec, from: NaiveDate, anchoring: Anchoring) -> NaiveDate {
    // An explicit `from DATE` always wins, for both callers.
    if let Some(start) = spec.start {
        return start;
    }
    if let Some(dom) = spec.day_of_month {
        return first_day_of_month_on_or_after(from, dom);
    }
    if let Some(w) = spec.weekday {
        let delta = (7 + w.num_days_from_monday() as i64
            - from.weekday().num_days_from_monday() as i64)
            % 7;
        return from + Duration::days(delta);
    }
    match anchoring {
        Anchoring::Grid => align_to_unit(from, spec.unit),
        Anchoring::WindowStart => from,
    }
}

/// Day `dom` of the given month, clamped to the last day when the month is
/// too short. Verified against hledger 1.50.3: `every 31st day of month`
/// yields 2024-01-31, 2024-02-29, 2024-03-31, 2024-04-30 — short months are
/// clamped, not skipped.
fn day_of_month_clamped(year: i32, month: u32, dom: u32) -> Option<NaiveDate> {
    let last = last_day_of_month(year, month)?;
    NaiveDate::from_ymd_opt(year, month, dom.min(last))
}

fn last_day_of_month(year: i32, month: u32) -> Option<u32> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    Some(add_months(first, 1).pred_opt()?.day())
}

/// The first date on or after `from` falling on day-of-month `dom`.
fn first_day_of_month_on_or_after(from: NaiveDate, dom: u32) -> NaiveDate {
    let mut month_start = NaiveDate::from_ymd_opt(from.year(), from.month(), 1).unwrap_or(from);
    for _ in 0..24 {
        if let Some(candidate) =
            day_of_month_clamped(month_start.year(), month_start.month(), dom)
        {
            if candidate >= from {
                return candidate;
            }
        }
        month_start = add_months(month_start, 1);
    }
    from
}

/// Occurrence dates in `[from, to]` on the natural calendar grid — budget
/// semantics. See [`Anchoring`].
pub fn occurrences(spec: &PeriodSpec, from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    occurrences_with(spec, from, to, Anchoring::Grid)
}

/// Occurrence dates in `[window_start, window_end]` with an unanchored rule
/// inheriting the window's start — forecast semantics, matching hledger.
pub fn forecast_occurrences(
    spec: &PeriodSpec,
    window_start: NaiveDate,
    window_end: NaiveDate,
) -> Vec<NaiveDate> {
    occurrences_with(spec, window_start, window_end, Anchoring::WindowStart)
}

/// Occurrence dates in `[window_start, window_end]`, respecting the spec's own
/// `from`/`to` bounds. `to` is exclusive, matching hledger.
pub fn occurrences_with(
    spec: &PeriodSpec,
    window_start: NaiveDate,
    window_end: NaiveDate,
    anchoring: Anchoring,
) -> Vec<NaiveDate> {
    let mut dates = Vec::new();
    if window_end < window_start {
        return dates;
    }

    let mut current = anchor_for(spec, window_start, anchoring);
    let mut guard = 0u32;

    // A `from` date before the window: fast-forward onto the grid.
    while current < window_start {
        match next_occurrence(spec, current) {
            Some(next) if next > current => current = next,
            _ => return dates,
        }
        guard += 1;
        if guard > MAX_STEPS {
            return dates;
        }
    }

    while current <= window_end {
        if let Some(end) = spec.end {
            if current >= end {
                break;
            }
        }
        dates.push(current);
        match next_occurrence(spec, current) {
            Some(next) if next > current => current = next,
            _ => break,
        }
        guard += 1;
        if guard > MAX_STEPS {
            break;
        }
    }

    dates
}

/// The occurrence after `date`, preserving any day-of-month anchor across
/// months of differing length.
fn next_occurrence(spec: &PeriodSpec, date: NaiveDate) -> Option<NaiveDate> {
    if let Some(dom) = spec.day_of_month {
        let month_start = NaiveDate::from_ymd_opt(date.year(), date.month(), 1)?;
        let next_month = add_months(month_start, spec.every.max(1));
        return day_of_month_clamped(next_month.year(), next_month.month(), dom);
    }
    step(date, spec.unit, spec.every)
}

/// How many occurrences fall in `[from, to]`.
pub fn count_occurrences(spec: &PeriodSpec, from: NaiveDate, to: NaiveDate) -> u32 {
    occurrences(spec, from, to).len() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn floating_monthly_anchors_to_window_start() {
        // Forecast semantics.
        // Verified against hledger 1.50.3:
        //   hledger print --forecast=2024-03-15..2024-07-01
        //   -> Rent on 2024-03-15, 04-15, 05-15, 06-15
        let spec = parse_period_expression("monthly").unwrap();
        assert!(spec.is_floating());
        let dates = forecast_occurrences(&spec, d(2024, 3, 15), d(2024, 6, 30));
        assert_eq!(
            dates,
            vec![d(2024, 3, 15), d(2024, 4, 15), d(2024, 5, 15), d(2024, 6, 15)]
        );

        // Budget semantics for the same rule stay on the calendar grid, so a
        // mid-month range only counts the occurrences inside it.
        let grid = occurrences(&spec, d(2024, 3, 15), d(2024, 6, 30));
        assert_eq!(grid, vec![d(2024, 4, 1), d(2024, 5, 1), d(2024, 6, 1)]);
    }

    #[test]
    fn explicit_from_keeps_its_own_grid() {
        // hledger: `~ every 2 weeks from 2024-01-05` over a window opening
        // 2024-03-10 still lands on the 01-05 fortnight grid.
        let spec = parse_period_expression("every 2 weeks from 2024-01-05").unwrap();
        let dates = forecast_occurrences(&spec, d(2024, 3, 10), d(2024, 4, 1));
        assert_eq!(dates, vec![d(2024, 3, 15), d(2024, 3, 29)]);
    }

    #[test]
    fn day_of_month_rule() {
        let spec = parse_period_expression("every 15th day of month").unwrap();
        assert_eq!(spec.day_of_month, Some(15));
        let dates = occurrences(&spec, d(2024, 2, 1), d(2024, 4, 30));
        assert_eq!(dates, vec![d(2024, 2, 15), d(2024, 3, 15), d(2024, 4, 15)]);
    }

    #[test]
    fn day_of_month_clamps_in_short_months() {
        // Verified against hledger 1.50.3:
        //   ~ every 31st day of month, --forecast=2024-01-01..2024-06-01
        //   -> 01-31, 02-29, 03-31, 04-30, 05-31
        // Short months clamp to their last day rather than being skipped.
        let spec = parse_period_expression("every 31st day of month").unwrap();
        let dates = occurrences(&spec, d(2024, 1, 1), d(2024, 5, 31));
        assert_eq!(
            dates,
            vec![
                d(2024, 1, 31),
                d(2024, 2, 29),
                d(2024, 3, 31),
                d(2024, 4, 30),
                d(2024, 5, 31)
            ]
        );
    }

    #[test]
    fn weekday_rule() {
        // 2024-02-02 is a Friday.
        let spec = parse_period_expression("every friday").unwrap();
        let dates = occurrences(&spec, d(2024, 2, 1), d(2024, 2, 29));
        assert_eq!(
            dates,
            vec![d(2024, 2, 2), d(2024, 2, 9), d(2024, 2, 16), d(2024, 2, 23)]
        );
    }

    #[test]
    fn to_bound_is_exclusive() {
        let spec = parse_period_expression("monthly from 2024-03-01 to 2024-05-01").unwrap();
        let dates = occurrences(&spec, d(2024, 1, 1), d(2024, 12, 31));
        assert_eq!(dates, vec![d(2024, 3, 1), d(2024, 4, 1)]);
    }

    #[test]
    fn unsupported_expressions_are_rejected() {
        for input in [
            "fortnightly",
            "every 0 weeks",
            "every 2nd monday of month",
            "nonsense",
            "every 45th day of month",
        ] {
            assert!(
                parse_period_expression(input).is_none(),
                "accepted {input:?}"
            );
        }
    }

    #[test]
    fn ordinals_parse() {
        assert_eq!(parse_ordinal("1st"), Some(1));
        assert_eq!(parse_ordinal("2nd"), Some(2));
        assert_eq!(parse_ordinal("15th"), Some(15));
        assert_eq!(parse_ordinal("monday"), None);
        assert_eq!(parse_ordinal("15"), None);
    }

    #[test]
    fn count_matches_occurrence_list() {
        let spec = parse_period_expression("monthly from 2024-01-01").unwrap();
        assert_eq!(count_occurrences(&spec, d(2024, 1, 1), d(2024, 6, 30)), 6);
    }
}
