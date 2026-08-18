use chrono::NaiveDate;

use crate::error::ParseError;

/// Parse a full date in YYYY-MM-DD, YYYY/MM/DD, or YYYY.MM.DD format.
pub fn parse_date(s: &str) -> Result<NaiveDate, ParseError> {
    parse_date_with_year(s, None)
}

/// Parse a date, allowing a partial MM-DD form when a default year is known
/// (from a `Y`/`year` directive, or the primary date for secondary dates).
pub fn parse_date_with_year(s: &str, default_year: Option<i32>) -> Result<NaiveDate, ParseError> {
    let s = s.trim();

    // Normalize separators to '-' for uniform parsing.
    let normalized: String = s
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect();

    let parts: Vec<&str> = normalized.split('-').collect();
    let all_numeric = parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if !all_numeric {
        return Err(ParseError::InvalidDate(s.to_string()));
    }

    match parts.len() {
        3 => {
            let y: i32 = parts[0].parse().map_err(|_| ParseError::InvalidDate(s.to_string()))?;
            let m: u32 = parts[1].parse().map_err(|_| ParseError::InvalidDate(s.to_string()))?;
            let d: u32 = parts[2].parse().map_err(|_| ParseError::InvalidDate(s.to_string()))?;
            // Reject 2-digit "years" like 12-31-05 being read as year 12.
            if parts[0].len() < 4 {
                return Err(ParseError::InvalidDate(s.to_string()));
            }
            NaiveDate::from_ymd_opt(y, m, d).ok_or_else(|| ParseError::InvalidDate(s.to_string()))
        }
        2 => {
            let year = default_year.ok_or_else(|| {
                ParseError::InvalidDate(format!(
                    "{} (partial date needs a Y/year directive)",
                    s
                ))
            })?;
            let m: u32 = parts[0].parse().map_err(|_| ParseError::InvalidDate(s.to_string()))?;
            let d: u32 = parts[1].parse().map_err(|_| ParseError::InvalidDate(s.to_string()))?;
            NaiveDate::from_ymd_opt(year, m, d)
                .ok_or_else(|| ParseError::InvalidDate(s.to_string()))
        }
        _ => Err(ParseError::InvalidDate(s.to_string())),
    }
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
    fn parse_partial_date_with_year() {
        assert_eq!(
            parse_date_with_year("01-15", Some(2024)).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
        assert_eq!(
            parse_date_with_year("1/15", Some(2024)).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
    }

    #[test]
    fn parse_partial_date_without_year_fails() {
        assert!(parse_date_with_year("01-15", None).is_err());
    }

    #[test]
    fn parse_date_invalid() {
        assert!(parse_date("not-a-date").is_err());
        assert!(parse_date("2024-13-01").is_err());
        assert!(parse_date("").is_err());
        assert!(parse_date("12345678").is_err());
    }
}
