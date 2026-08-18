use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;

use crate::ast::{AmountStyle, PostingAmount, Side};
use crate::error::ParseError;

/// Context that influences how numbers are read: the file's `decimal-mark`
/// directive, per-commodity styles from `commodity` directives, and the `D`
/// default commodity.
#[derive(Debug, Clone, Default)]
pub struct AmountContext {
    /// From a `decimal-mark` directive: forces the decimal mark for the file.
    pub decimal_mark: Option<char>,
    /// Per-commodity decimal marks from `commodity` directive formats.
    pub commodity_marks: HashMap<String, char>,
    /// From a `D` directive: commodity + style applied to bare numbers.
    pub default_commodity: Option<(String, AmountStyle)>,
}

impl AmountContext {
    fn mark_for(&self, commodity: &str) -> Option<char> {
        self.commodity_marks
            .get(commodity)
            .copied()
            .or(self.decimal_mark)
    }
}

/// Result of parsing a numeric quantity, retaining the display style seen in
/// the source so rewrites can preserve it.
#[derive(Debug, Clone, Copy)]
pub struct ParsedQuantity {
    pub value: Decimal,
    pub decimal_mark: char,
    pub precision: u8,
}

/// Parse an amount string like "$100.00", "100.00 USD", "-€50", "1.234,56 EUR".
pub fn parse_amount(s: &str) -> Result<PostingAmount, ParseError> {
    parse_amount_ctx(s, &AmountContext::default())
}

/// Parse an amount with number-format context.
pub fn parse_amount_ctx(s: &str, ctx: &AmountContext) -> Result<PostingAmount, ParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ParseError::InvalidAmount("empty amount".to_string()));
    }

    // Sign written before the commodity symbol: -$50, +€10
    if let Some(first) = s.chars().next() {
        if (first == '-' || first == '+') && !s[1..].trim_start().starts_with(|c: char| c.is_ascii_digit() || c == '.' || c == ',') {
            let rest = s[1..].trim_start();
            let mut amt = parse_amount_ctx(rest, ctx)?;
            if first == '-' {
                amt.quantity = -amt.quantity;
            }
            return Ok(amt);
        }
    }

    // Quoted commodity on the left: "ABC DEF" 100
    if let Some(rest) = s.strip_prefix('"') {
        if let Some(close) = rest.find('"') {
            let commodity = &rest[..close];
            let num = rest[close + 1..].trim_start();
            let spaced = rest[close + 1..].starts_with(' ');
            let q = parse_quantity_with(num, ctx.mark_for(commodity))?;
            return Ok(make_amount(q, commodity, Side::Left, spaced));
        }
    }

    // Quoted commodity on the right: 100 "ABC DEF"
    if s.ends_with('"') {
        let body = &s[..s.len() - 1];
        if let Some(open) = body.rfind('"') {
            let commodity = &body[open + 1..];
            let num = body[..open].trim_end();
            let spaced = body[..open].ends_with(' ');
            let q = parse_quantity_with(num, ctx.mark_for(commodity))?;
            return Ok(make_amount(q, commodity, Side::Right, spaced));
        }
    }

    // Commodity on the left: $100.00, €50, CAD 100.00, USD100.00, zł50
    if let Some(result) = try_left_commodity(s, ctx) {
        return Ok(result);
    }

    // Commodity on the right: 100.00 USD, 10 AAPL, 50zł
    if let Some(result) = try_right_commodity(s, ctx) {
        return Ok(result);
    }

    // Bare number (no commodity): apply the D default commodity if set.
    let q = parse_quantity_with(s, ctx.decimal_mark)?;
    if let Some((commodity, style)) = &ctx.default_commodity {
        let mut st = style.clone();
        st.decimal_mark = q.decimal_mark;
        st.precision = q.precision;
        return Ok(PostingAmount {
            quantity: q.value,
            commodity: commodity.clone(),
            style: st,
            cost: None,
            multiplier: false,
        });
    }
    Ok(PostingAmount {
        quantity: q.value,
        commodity: String::new(),
        style: AmountStyle {
            commodity_side: Side::Left,
            commodity_spaced: false,
            decimal_mark: q.decimal_mark,
            precision: q.precision,
        },
        cost: None,
        multiplier: false,
    })
}

fn make_amount(q: ParsedQuantity, commodity: &str, side: Side, spaced: bool) -> PostingAmount {
    PostingAmount {
        quantity: q.value,
        commodity: commodity.to_string(),
        style: AmountStyle {
            commodity_side: side,
            commodity_spaced: spaced,
            decimal_mark: q.decimal_mark,
            precision: q.precision,
        },
        cost: None,
        multiplier: false,
    }
}

fn try_left_commodity(s: &str, ctx: &AmountContext) -> Option<PostingAmount> {
    // Single-char currency symbols: $, €, £, ¥, ...
    let first_char = s.chars().next()?;
    if is_currency_symbol(first_char) {
        let rest = &s[first_char.len_utf8()..];
        let (rest, spaced) = if rest.starts_with(' ') {
            (rest.trim_start(), true)
        } else {
            (rest, false)
        };
        let commodity = first_char.to_string();
        let q = parse_quantity_with(rest, ctx.mark_for(&commodity)).ok()?;
        return Some(make_amount(q, &commodity, Side::Left, spaced));
    }

    // Alphabetic commodity before number: CAD 100.00, USD100.00, zł50
    let commodity_end = s
        .char_indices()
        .find(|(_, c)| !c.is_alphabetic())
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    if commodity_end == 0 || commodity_end == s.len() {
        return None;
    }

    let commodity = &s[..commodity_end];
    let rest = &s[commodity_end..];
    let (rest, spaced) = if rest.starts_with(' ') {
        (rest.trim_start(), true)
    } else {
        (rest, false)
    };

    let q = parse_quantity_with(rest, ctx.mark_for(commodity)).ok()?;
    Some(make_amount(q, commodity, Side::Left, spaced))
}

fn try_right_commodity(s: &str, ctx: &AmountContext) -> Option<PostingAmount> {
    // Space-separated: 100.00 USD
    if let Some(last_space) = s.rfind(' ') {
        let number_part = s[..last_space].trim();
        let commodity_part = s[last_space + 1..].trim();
        if !commodity_part.is_empty() && !number_part.is_empty() {
            let first = commodity_part.chars().next()?;
            if first.is_alphabetic() || is_currency_symbol(first) {
                let q = parse_quantity_with(number_part, ctx.mark_for(commodity_part)).ok()?;
                return Some(make_amount(q, commodity_part, Side::Right, true));
            }
        }
        return None;
    }

    // Attached: 50zł, 100kr — trailing run of alphabetic or symbol chars.
    let commodity_start = s
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphabetic() || is_currency_symbol(*c))
        .last()
        .map(|(i, _)| i)?;
    if commodity_start == 0 {
        return None;
    }
    let number_part = &s[..commodity_start];
    let commodity_part = &s[commodity_start..];
    let q = parse_quantity_with(number_part, ctx.mark_for(commodity_part)).ok()?;
    Some(make_amount(q, commodity_part, Side::Right, false))
}

fn is_currency_symbol(c: char) -> bool {
    matches!(c, '$' | '€' | '£' | '¥' | '₹' | '₽' | '₿' | '₩' | '₫' | '₴' | '₸' | '₺' | '₦' | '₭')
}

/// Parse a numeric quantity with hledger's decimal-mark semantics.
///
/// With an explicit mark (from `decimal-mark` or a commodity style), that char
/// is the decimal mark and the other of `.`/`,` is a digit group mark.
/// Without one, infer like hledger:
/// - both `.` and `,` present: the rightmost is the decimal mark;
/// - one kind present multiple times: digit group marks;
/// - one kind present once: the decimal mark (hledger's documented default).
pub fn parse_quantity(s: &str) -> Result<Decimal, ParseError> {
    parse_quantity_with(s, None).map(|q| q.value)
}

pub fn parse_quantity_with(
    s: &str,
    decimal_mark: Option<char>,
) -> Result<ParsedQuantity, ParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ParseError::InvalidAmount("empty quantity".to_string()));
    }

    let dots = s.matches('.').count();
    let commas = s.matches(',').count();

    let mark: Option<char> = match decimal_mark {
        Some(m) => Some(m),
        None => {
            if dots > 0 && commas > 0 {
                // Rightmost separator is the decimal mark.
                let last_dot = s.rfind('.').unwrap();
                let last_comma = s.rfind(',').unwrap();
                Some(if last_dot > last_comma { '.' } else { ',' })
            } else if dots == 1 {
                Some('.')
            } else if commas == 1 {
                Some(',')
            } else {
                // Only repeated separators of one kind (or none): group marks.
                None
            }
        }
    };

    // A separator appearing more than once cannot be the decimal mark even if
    // the context says so (e.g. "1.000.000" with decimal-mark '.') — treat the
    // string as invalid rather than guess.
    if let Some(m) = mark {
        let count = s.matches(m).count();
        if count > 1 {
            return Err(ParseError::InvalidAmount(format!(
                "invalid number (repeated decimal mark '{}'): {}",
                m, s
            )));
        }
    }

    let mut cleaned = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '.' | ',' | ' ' | '\u{a0}' | '\u{202f}' | '\'' => {
                if Some(c) == mark {
                    cleaned.push('.');
                }
                // else: digit group mark, drop it
            }
            _ => cleaned.push(c),
        }
    }

    let precision = match mark.and_then(|m| s.rfind(m).map(|p| (m, p))) {
        Some((m, pos)) => s[pos + m.len_utf8()..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .count()
            .min(255) as u8,
        None => 0,
    };

    let value = Decimal::from_str(&cleaned)
        .or_else(|_| {
            if cleaned.contains(['e', 'E']) {
                Decimal::from_scientific(&cleaned)
            } else {
                Decimal::from_str(&cleaned)
            }
        })
        .map_err(|_| ParseError::InvalidAmount(format!("invalid number: {}", s)))?;

    Ok(ParsedQuantity {
        value,
        decimal_mark: mark.unwrap_or('.'),
        precision,
    })
}

/// Detect the display style of an example amount (used by `commodity` and `D`
/// directives). Returns (commodity, style).
pub fn parse_style_example(s: &str) -> Option<(String, AmountStyle)> {
    let amt = parse_amount_ctx(s.trim(), &AmountContext::default()).ok()?;
    Some((amt.commodity, amt.style))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn parse_dollar_amount() {
        let amt = parse_amount("$100.00").unwrap();
        assert_eq!(amt.quantity, dec!(100.00));
        assert_eq!(amt.commodity, "$");
        assert_eq!(amt.style.commodity_side, Side::Left);
        assert!(!amt.style.commodity_spaced);
        assert_eq!(amt.style.precision, 2);
    }

    #[test]
    fn parse_negative_dollar() {
        let amt = parse_amount("$-50.25").unwrap();
        assert_eq!(amt.quantity, dec!(-50.25));
        assert_eq!(amt.commodity, "$");
    }

    #[test]
    fn parse_sign_before_symbol() {
        let amt = parse_amount("-$50.00").unwrap();
        assert_eq!(amt.quantity, dec!(-50.00));
        assert_eq!(amt.commodity, "$");

        let amt = parse_amount("-€50").unwrap();
        assert_eq!(amt.quantity, dec!(-50));

        let amt = parse_amount("+$10").unwrap();
        assert_eq!(amt.quantity, dec!(10));
    }

    #[test]
    fn parse_euro_right() {
        let amt = parse_amount("100.00 EUR").unwrap();
        assert_eq!(amt.quantity, dec!(100.00));
        assert_eq!(amt.commodity, "EUR");
        assert_eq!(amt.style.commodity_side, Side::Right);
    }

    #[test]
    fn parse_commodity_stock() {
        let amt = parse_amount("10 AAPL").unwrap();
        assert_eq!(amt.quantity, dec!(10));
        assert_eq!(amt.commodity, "AAPL");
        assert_eq!(amt.style.precision, 0);
    }

    #[test]
    fn parse_negative_right_commodity() {
        let amt = parse_amount("-100.00 USD").unwrap();
        assert_eq!(amt.quantity, dec!(-100.00));
        assert_eq!(amt.commodity, "USD");
    }

    #[test]
    fn parse_with_thousands_separator() {
        let amt = parse_amount("$1,000.00").unwrap();
        assert_eq!(amt.quantity, dec!(1000.00));
    }

    #[test]
    fn parse_indian_grouping() {
        let amt = parse_amount("1,00,000.00 INR").unwrap();
        assert_eq!(amt.quantity, dec!(100000.00));
    }

    #[test]
    fn parse_comma_decimal_both_separators() {
        // Both present: rightmost is the decimal mark.
        let amt = parse_amount("1.234,56 EUR").unwrap();
        assert_eq!(amt.quantity, dec!(1234.56));
        assert_eq!(amt.style.decimal_mark, ',');
        assert_eq!(amt.style.precision, 2);
    }

    #[test]
    fn parse_lone_comma_is_decimal() {
        // hledger: a single separator is a decimal mark.
        let amt = parse_amount("3,50 EUR").unwrap();
        assert_eq!(amt.quantity, dec!(3.50));

        let amt = parse_amount("1,234 EUR").unwrap();
        assert_eq!(amt.quantity, dec!(1.234));
    }

    #[test]
    fn parse_repeated_commas_are_grouping() {
        let amt = parse_amount("1,000,000 USD").unwrap();
        assert_eq!(amt.quantity, dec!(1000000));
    }

    #[test]
    fn parse_with_explicit_decimal_mark() {
        let mut ctx = AmountContext::default();
        ctx.decimal_mark = Some(',');
        let amt = parse_amount_ctx("1.234,56 EUR", &ctx).unwrap();
        assert_eq!(amt.quantity, dec!(1234.56));
        // With comma as decimal mark, a lone period is grouping.
        let amt = parse_amount_ctx("1.234 EUR", &ctx).unwrap();
        assert_eq!(amt.quantity, dec!(1234));
    }

    #[test]
    fn parse_commodity_specific_mark() {
        let mut ctx = AmountContext::default();
        ctx.commodity_marks.insert("EUR".to_string(), ',');
        let amt = parse_amount_ctx("1.234 EUR", &ctx).unwrap();
        assert_eq!(amt.quantity, dec!(1234));
        // Other commodities fall back to inference.
        let amt = parse_amount_ctx("1.234 USD", &ctx).unwrap();
        assert_eq!(amt.quantity, dec!(1.234));
    }

    #[test]
    fn parse_repeated_decimal_mark_rejected() {
        let mut ctx = AmountContext::default();
        ctx.decimal_mark = Some('.');
        assert!(parse_amount_ctx("1.000.000 USD", &ctx).is_err());
    }

    #[test]
    fn parse_euro_symbol_left() {
        let amt = parse_amount("€50").unwrap();
        assert_eq!(amt.quantity, dec!(50));
        assert_eq!(amt.commodity, "€");
        assert_eq!(amt.style.commodity_side, Side::Left);
    }

    #[test]
    fn parse_bare_number() {
        let amt = parse_amount("42.50").unwrap();
        assert_eq!(amt.quantity, dec!(42.50));
        assert_eq!(amt.commodity, "");
    }

    #[test]
    fn parse_default_commodity_applied() {
        let mut ctx = AmountContext::default();
        ctx.default_commodity = Some((
            "$".to_string(),
            AmountStyle {
                commodity_side: Side::Left,
                commodity_spaced: false,
                decimal_mark: '.',
                precision: 2,
            },
        ));
        let amt = parse_amount_ctx("25", &ctx).unwrap();
        assert_eq!(amt.commodity, "$");
        assert_eq!(amt.quantity, dec!(25));
    }

    #[test]
    fn parse_empty_fails() {
        assert!(parse_amount("").is_err());
    }

    #[test]
    fn parse_left_commodity_spaced() {
        let amt = parse_amount("CAD 100.00").unwrap();
        assert_eq!(amt.quantity, dec!(100.00));
        assert_eq!(amt.commodity, "CAD");
        assert_eq!(amt.style.commodity_side, Side::Left);
        assert!(amt.style.commodity_spaced);
    }

    #[test]
    fn parse_adjacent_left_commodity() {
        let amt = parse_amount("USD100.00").unwrap();
        assert_eq!(amt.quantity, dec!(100.00));
        assert_eq!(amt.commodity, "USD");
        assert!(!amt.style.commodity_spaced);
    }

    #[test]
    fn parse_nonascii_left_commodity() {
        let amt = parse_amount("zł50").unwrap();
        assert_eq!(amt.quantity, dec!(50));
        assert_eq!(amt.commodity, "zł");
    }

    #[test]
    fn parse_adjacent_right_commodity() {
        let amt = parse_amount("50kr").unwrap();
        assert_eq!(amt.quantity, dec!(50));
        assert_eq!(amt.commodity, "kr");
        assert_eq!(amt.style.commodity_side, Side::Right);
    }

    #[test]
    fn parse_quoted_commodity() {
        let amt = parse_amount("1.5 \"VANGUARD FUND\"").unwrap();
        assert_eq!(amt.quantity, dec!(1.5));
        assert_eq!(amt.commodity, "VANGUARD FUND");
        assert_eq!(amt.style.commodity_side, Side::Right);

        let amt = parse_amount("\"AAPL 2024\" 10").unwrap();
        assert_eq!(amt.quantity, dec!(10));
        assert_eq!(amt.commodity, "AAPL 2024");
    }

    #[test]
    fn parse_scientific_notation() {
        let amt = parse_amount("1E3 USD").unwrap();
        assert_eq!(amt.quantity, dec!(1000));
    }

    #[test]
    fn parse_high_precision() {
        let amt = parse_amount("0.00012345 BTC").unwrap();
        assert_eq!(amt.quantity, dec!(0.00012345));
        assert_eq!(amt.style.precision, 8);
    }
}
