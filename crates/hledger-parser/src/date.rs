use chrono::NaiveDate;

use crate::error::ParseError;

/// Parse a date string in YYYY-MM-DD, YYYY/MM/DD, or YYYY.MM.DD format.
pub fn parse_date(s: &str) -> Result<NaiveDate, ParseError> {
    // Try all three separators
    let s = s.trim();

    if s.len() < 8 {
        return Err(ParseError::InvalidDate(s.to_string()));
    }

    // Replace separators with '-' for uniform parsing
    let normalized: String = s
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect();

    NaiveDate::parse_from_str(&normalized, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(&normalized, "%Y-%-m-%-d"))
        .map_err(|_| ParseError::InvalidDate(s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn parse_date_dashes() {
        assert_eq!(
            parse_date("2024-01-15").unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
    }

    #[test]
    fn parse_date_slashes() {
        assert_eq!(
            parse_date("2024/01/15").unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
    }

    #[test]
    fn parse_date_dots() {
        assert_eq!(
            parse_date("2024.01.15").unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
    }

    #[test]
    fn parse_date_no_leading_zeros() {
        assert_eq!(
            parse_date("2024-1-5").unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()
        );
    }

    #[test]
    fn parse_date_invalid() {
        assert!(parse_date("not-a-date").is_err());
        assert!(parse_date("2024-13-01").is_err());
        assert!(parse_date("").is_err());
    }
}
