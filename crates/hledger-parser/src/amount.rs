use rust_decimal::Decimal;
use std::str::FromStr;

use crate::ast::{AmountStyle, PostingAmount, Side};
use crate::error::ParseError;

/// Parse an amount string like "$100.00", "100.00 USD", "€50", "10 AAPL".
/// Returns (quantity, commodity, style).
pub fn parse_amount(s: &str) -> Result<PostingAmount, ParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ParseError::InvalidAmount("empty amount".to_string()));
    }

    // Try commodity on left: $100.00, €50, CAD 100.00
    if let Some(result) = try_left_commodity(s) {
        return Ok(result);
    }

    // Try commodity on right: 100.00 USD, 10 AAPL
    if let Some(result) = try_right_commodity(s) {
        return Ok(result);
    }

    // Try bare number (no commodity)
    let quantity = parse_quantity(s)?;
    Ok(PostingAmount {
        quantity,
        commodity: String::new(),
        style: AmountStyle::default(),
        cost: None,
    })
}

fn try_left_commodity(s: &str) -> Option<PostingAmount> {
    // Single-char symbols: $, €, £, ¥
    let first_char = s.chars().next()?;
    if is_currency_symbol(first_char) {
        let rest = &s[first_char.len_utf8()..];
        let (rest, spaced) = if rest.starts_with(' ') {
            (rest.trim_start(), true)
        } else {
            (rest, false)
        };

        let quantity = parse_quantity(rest).ok()?;
        let precision = decimal_precision(rest);

        return Some(PostingAmount {
            quantity,
            commodity: first_char.to_string(),
            style: AmountStyle {
                commodity_side: Side::Left,
                commodity_spaced: spaced,
                decimal_mark: '.',
                precision,
            },
            cost: None,
        });
    }

    // Multi-char commodity before number: CAD 100.00
    // Look for leading alphabetic characters
    let commodity_end = s.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(s.len());
    if commodity_end == 0 || commodity_end == s.len() {
        return None;
    }

    let commodity = &s[..commodity_end];
    let rest = &s[commodity_end..];
    let (rest, spaced) = if rest.starts_with(' ') {
        (rest.trim_start(), true)
    } else {
        return None; // Multi-char left commodity must have space
    };

    let quantity = parse_quantity(rest).ok()?;
    let precision = decimal_precision(rest);

    Some(PostingAmount {
        quantity,
        commodity: commodity.to_string(),
        style: AmountStyle {
            commodity_side: Side::Left,
            commodity_spaced: spaced,
            decimal_mark: '.',
            precision,
        },
        cost: None,
    })
}

fn try_right_commodity(s: &str) -> Option<PostingAmount> {
    // Find last space that separates number from commodity
    // Handle negative numbers: -100.00 USD
    let last_space = s.rfind(' ')?;
    let number_part = s[..last_space].trim();
    let commodity_part = s[last_space + 1..].trim();

    if commodity_part.is_empty() || number_part.is_empty() {
        return None;
    }

    // Commodity must start with a letter or symbol
    let first = commodity_part.chars().next()?;
    if !first.is_alphabetic() && !is_currency_symbol(first) {
        return None;
    }

    let quantity = parse_quantity(number_part).ok()?;
    let precision = decimal_precision(number_part);

    Some(PostingAmount {
        quantity,
        commodity: commodity_part.to_string(),
        style: AmountStyle {
            commodity_side: Side::Right,
            commodity_spaced: true,
            decimal_mark: '.',
            precision,
        },
        cost: None,
    })
}

fn is_currency_symbol(c: char) -> bool {
    matches!(c, '$' | '€' | '£' | '¥' | '₹' | '₽' | '₿' | '₩' | '₫' | '₴' | '₸' | '₺' | '₦' | '₭')
}

/// Parse a numeric quantity, stripping thousand separators.
pub fn parse_quantity(s: &str) -> Result<Decimal, ParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ParseError::InvalidAmount("empty quantity".to_string()));
    }

    // Strip thousand separators (commas when period is decimal, or periods when comma is decimal)
    // Default: period is decimal mark
    let cleaned: String = s.chars().filter(|&c| c != ',').collect();

    Decimal::from_str(&cleaned)
        .map_err(|_| ParseError::InvalidAmount(format!("invalid number: {}", s)))
}

/// Count decimal places in a numeric string.
fn decimal_precision(s: &str) -> u8 {
    if let Some(dot_pos) = s.rfind('.') {
        let after_dot = &s[dot_pos + 1..];
        // Count only digits after the decimal
        after_dot
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .count() as u8
    } else {
        0
    }
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
}
