//! hledger's query language: `acct:`, `desc:`, `amt:`, `date:`, `cur:`,
//! `status:`, `tag:`, `code:`, `payee:`, `not:` prefixes, quoted terms.
//!
//! Semantics follow hledger: terms of the same type OR together, different
//! types AND together, `not:` terms exclude. One deliberate deviation for
//! mobile UX: a bare term (no prefix) matches account OR description,
//! case-insensitively (hledger matches accounts only; `acct:` gives the
//! exact hledger behavior).

use chrono::NaiveDate;
use regex::Regex;
use rust_decimal::Decimal;

use hledger_parser::ast::Status;

use crate::balance::{ResolvedPosting, ResolvedTransaction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

#[derive(Debug, Clone)]
enum Term {
    /// Bare term: account or description regex (UX superset of hledger).
    Any(Regex),
    Account(Regex),
    Description(Regex),
    Code(Regex),
    Note(Regex),
    Tag { name: Regex, value: Option<Regex> },
    /// Magnitude comparison when `abs` is true (unsigned query value).
    Amount { op: Cmp, value: Decimal, abs: bool },
    Commodity(Regex),
    Status(Status),
    Date { from: Option<NaiveDate>, to: Option<NaiveDate> },
    Real(bool),
    Depth(usize),
}

impl Term {
    /// Group key: same-type terms OR together.
    fn kind(&self) -> u8 {
        match self {
            Term::Any(_) => 0,
            Term::Account(_) => 1,
            Term::Description(_) => 2,
            Term::Code(_) => 3,
            Term::Note(_) => 4,
            Term::Tag { .. } => 5,
            Term::Amount { .. } => 6,
            Term::Commodity(_) => 7,
            Term::Status(_) => 8,
            Term::Date { .. } => 9,
            Term::Real(_) => 10,
            Term::Depth(_) => 11,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Query {
    /// Positive terms grouped by kind: a posting must match at least one term
    /// of every group.
    groups: Vec<Vec<Term>>,
    /// `not:` terms: a posting matching any of these is excluded.
    negatives: Vec<Term>,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.negatives.is_empty()
    }

    /// Depth limit, if the query carries one (used by reports).
    pub fn depth(&self) -> Option<usize> {
        for group in &self.groups {
            for term in group {
                if let Term::Depth(d) = term {
                    return Some(*d);
                }
            }
        }
        None
    }

    /// Does this posting (in its transaction) match?
    pub fn matches_posting(&self, txn: &ResolvedTransaction, posting: &ResolvedPosting) -> bool {
        for group in &self.groups {
            if !group.iter().any(|t| term_matches(t, txn, posting)) {
                return false;
            }
        }
        !self
            .negatives
            .iter()
            .any(|t| term_matches(t, txn, posting))
    }

    /// A transaction matches when any of its postings does (hledger print
    /// semantics).
    pub fn matches_transaction(&self, txn: &ResolvedTransaction) -> bool {
        if self.is_empty() {
            return true;
        }
        txn.postings.iter().any(|p| self.matches_posting(txn, p))
    }
}

fn term_matches(term: &Term, txn: &ResolvedTransaction, posting: &ResolvedPosting) -> bool {
    match term {
        Term::Any(re) => {
            re.is_match(&posting.account.full) || re.is_match(&txn.description)
        }
        Term::Account(re) => re.is_match(&posting.account.full),
        Term::Description(re) => re.is_match(&txn.description),
        Term::Code(re) => txn.code.as_deref().map_or(false, |c| re.is_match(c)),
        Term::Note(re) => {
            posting
                .comment
                .as_deref()
                .map_or(false, |c| re.is_match(c))
                || txn.comment.as_deref().map_or(false, |c| re.is_match(c))
        }
        Term::Tag { name, value } => {
            // Tags live on the comment text in resolved form; match against
            // "name:value" pairs found in txn+posting comments.
            let mut haystacks = Vec::new();
            if let Some(c) = &txn.comment {
                haystacks.push(c.as_str());
            }
            if let Some(c) = &posting.comment {
                haystacks.push(c.as_str());
            }
            haystacks.iter().any(|text| {
                for pair in extract_tag_pairs(text) {
                    if name.is_match(&pair.0) {
                        match value {
                            None => return true,
                            Some(vre) => {
                                if vre.is_match(&pair.1) {
                                    return true;
                                }
                            }
                        }
                    }
                }
                false
            })
        }
        Term::Amount { op, value, abs } => posting.amount.amounts.values().any(|q| {
            let q = if *abs { q.abs() } else { *q };
            match op {
                Cmp::Lt => q < *value,
                Cmp::Le => q <= *value,
                Cmp::Gt => q > *value,
                Cmp::Ge => q >= *value,
                Cmp::Eq => q == *value,
            }
        }),
        Term::Commodity(re) => posting
            .amount
            .amounts
            .keys()
            .any(|c| re.is_match(c)),
        Term::Status(s) => posting.status == *s,
        Term::Date { from, to } => {
            if let Some(f) = from {
                if posting.date < *f {
                    return false;
                }
            }
            if let Some(t) = to {
                if posting.date > *t {
                    return false;
                }
            }
            true
        }
        Term::Real(real) => posting.is_virtual != *real,
        Term::Depth(_) => true,
    }
}

/// Extract name:value pairs from resolved comment text (parser tag rules).
fn extract_tag_pairs(text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ':' {
            let mut start = i;
            while start > 0 {
                let c = chars[start - 1];
                if c.is_whitespace() || c == ',' || c == ':' {
                    break;
                }
                start -= 1;
            }
            if start < i {
                let name: String = chars[start..i].iter().collect();
                let mut vend = i + 1;
                while vend < chars.len() && chars[vend] != ',' {
                    vend += 1;
                }
                let value: String = chars[i + 1..vend]
                    .iter()
                    .collect::<String>()
                    .trim()
                    .to_string();
                pairs.push((name, value));
                i = vend + 1;
                continue;
            }
        }
        i += 1;
    }
    pairs
}

/// Parse a query string. Returns Err with a user-facing message for invalid
/// regexes or malformed terms — silent misinterpretation would filter wrongly.
/// Keep only the postings a query matches, dropping transactions left with
/// none.
///
/// Filtering at the posting level rather than the transaction level is what
/// makes `acct:expenses` mean "expense postings" instead of "every posting of
/// any transaction that touches an expense account" — the latter would drag
/// the funding side of each transaction into the totals.
pub fn retain_matching_postings(
    transactions: &[crate::balance::ResolvedTransaction],
    query: &Query,
) -> Vec<crate::balance::ResolvedTransaction> {
    transactions
        .iter()
        .filter_map(|txn| {
            let mut kept = txn.clone();
            kept.postings.retain(|p| query.matches_posting(txn, p));
            (!kept.postings.is_empty()).then_some(kept)
        })
        .collect()
}

pub fn parse_query(input: &str) -> Result<Query, String> {
    let mut query = Query::default();

    for raw in tokenize(input) {
        let (negated, body) = match raw.strip_prefix("not:") {
            Some(rest) => (true, rest.to_string()),
            None => (false, raw),
        };

        let term = parse_term(&body)?;
        if negated {
            query.negatives.push(term);
        } else {
            // Insert into the group of the same kind.
            let kind = term.kind();
            match query
                .groups
                .iter_mut()
                .find(|g| g.first().map(|t| t.kind()) == Some(kind))
            {
                Some(group) => group.push(term),
                None => query.groups.push(vec![term]),
            }
        }
    }

    Ok(query)
}

fn ci_regex(pattern: &str) -> Result<Regex, String> {
    Regex::new(&format!("(?i){}", pattern))
        .map_err(|e| format!("invalid regular expression '{}': {}", pattern, e))
}

fn parse_term(body: &str) -> Result<Term, String> {
    let (prefix, rest) = match body.split_once(':') {
        Some((p, r)) if is_known_prefix(p) => (p, r),
        _ => ("", body),
    };

    match prefix {
        "" => Ok(Term::Any(ci_regex(body)?)),
        "acct" => Ok(Term::Account(ci_regex(rest)?)),
        "desc" => Ok(Term::Description(ci_regex(rest)?)),
        "payee" => Ok(Term::Description(ci_regex(rest)?)),
        "code" => Ok(Term::Code(ci_regex(rest)?)),
        "note" => Ok(Term::Note(ci_regex(rest)?)),
        "tag" => {
            let (name, value) = match rest.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (rest, None),
            };
            Ok(Term::Tag {
                name: ci_regex(name)?,
                value: value.map(ci_regex).transpose()?,
            })
        }
        "cur" => {
            // hledger: cur: is a whole-symbol match.
            Ok(Term::Commodity(Regex::new(&format!("(?i)^(?:{})$", rest))
                .map_err(|e| format!("invalid cur: pattern '{}': {}", rest, e))?))
        }
        "amt" => parse_amount_term(rest),
        "status" => match rest {
            "*" => Ok(Term::Status(Status::Cleared)),
            "!" => Ok(Term::Status(Status::Pending)),
            "" => Ok(Term::Status(Status::Unmarked)),
            other => Err(format!("status: expects '*', '!' or nothing, got '{}'", other)),
        },
        "date" => parse_date_term(rest),
        "real" => match rest {
            "" | "1" | "true" => Ok(Term::Real(true)),
            "0" | "false" => Ok(Term::Real(false)),
            other => Err(format!("real: expects 1 or 0, got '{}'", other)),
        },
        "depth" => rest
            .parse::<usize>()
            .map(Term::Depth)
            .map_err(|_| format!("depth: expects a number, got '{}'", rest)),
        // Unreachable: only known prefixes are split off. Terms with other
        // colons (e.g. "assets:bank") are bare account/description regexes.
        _ => Ok(Term::Any(ci_regex(body)?)),
    }
}

fn is_known_prefix(p: &str) -> bool {
    matches!(
        p,
        "acct" | "desc" | "payee" | "code" | "note" | "tag" | "cur" | "amt" | "status" | "date"
            | "real" | "depth"
    )
}

fn parse_amount_term(rest: &str) -> Result<Term, String> {
    let (op, num) = if let Some(n) = rest.strip_prefix("<=") {
        (Cmp::Le, n)
    } else if let Some(n) = rest.strip_prefix(">=") {
        (Cmp::Ge, n)
    } else if let Some(n) = rest.strip_prefix('<') {
        (Cmp::Lt, n)
    } else if let Some(n) = rest.strip_prefix('>') {
        (Cmp::Gt, n)
    } else if let Some(n) = rest.strip_prefix('=') {
        (Cmp::Eq, n)
    } else {
        (Cmp::Eq, rest)
    };

    let num = num.trim();
    // Unsigned query value compares magnitudes (hledger semantics).
    let abs = !num.starts_with('-') && !num.starts_with('+');
    let value: Decimal = num
        .parse()
        .map_err(|_| format!("amt: expects a number, got '{}'", num))?;

    Ok(Term::Amount {
        op,
        value: if abs { value.abs() } else { value },
        abs,
    })
}

fn parse_date_term(rest: &str) -> Result<Term, String> {
    let parse_smart = |s: &str, end_of: bool| -> Result<NaiveDate, String> {
        let parts: Vec<&str> = s.split(['-', '/', '.']).collect();
        let bad = || format!("date: cannot parse '{}'", s);
        match parts.len() {
            1 => {
                let y: i32 = parts[0].parse().map_err(|_| bad())?;
                if end_of {
                    NaiveDate::from_ymd_opt(y, 12, 31).ok_or_else(bad)
                } else {
                    NaiveDate::from_ymd_opt(y, 1, 1).ok_or_else(bad)
                }
            }
            2 => {
                let y: i32 = parts[0].parse().map_err(|_| bad())?;
                let m: u32 = parts[1].parse().map_err(|_| bad())?;
                let first = NaiveDate::from_ymd_opt(y, m, 1).ok_or_else(bad)?;
                if end_of {
                    let next = if m == 12 {
                        NaiveDate::from_ymd_opt(y + 1, 1, 1)
                    } else {
                        NaiveDate::from_ymd_opt(y, m + 1, 1)
                    }
                    .ok_or_else(bad)?;
                    Ok(next.pred_opt().unwrap())
                } else {
                    Ok(first)
                }
            }
            3 => {
                let y: i32 = parts[0].parse().map_err(|_| bad())?;
                let m: u32 = parts[1].parse().map_err(|_| bad())?;
                let d: u32 = parts[2].parse().map_err(|_| bad())?;
                NaiveDate::from_ymd_opt(y, m, d).ok_or_else(bad)
            }
            _ => Err(bad()),
        }
    };

    if let Some((a, b)) = rest.split_once("..") {
        let from = if a.is_empty() {
            None
        } else {
            Some(parse_smart(a, false)?)
        };
        let to = if b.is_empty() {
            None
        } else {
            Some(parse_smart(b, true)?)
        };
        Ok(Term::Date { from, to })
    } else {
        Ok(Term::Date {
            from: Some(parse_smart(rest, false)?),
            to: Some(parse_smart(rest, true)?),
        })
    }
}

/// Split a query string on whitespace, honoring double and single quotes
/// (both `desc:"coffee shop"` and `"coffee shop"`).
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for c in input.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    current.push(c);
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                c if c.is_whitespace() => {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                }
                c => current.push(c),
            },
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::balance::resolve_transactions;
    use hledger_parser::parse;

    fn txns() -> Vec<ResolvedTransaction> {
        let input = "\
2024-01-05 * Coffee Shop ; category:fun
    expenses:food:coffee  $4.50
    assets:checking

2024-01-15 Rent payment
    expenses:rent  $1200.00
    assets:checking

2024-02-01 ! Salary
    assets:checking  2000 EUR
    income:salary  -2000 EUR
";
        resolve_transactions(&parse(input).unwrap()).unwrap()
    }

    fn matching<'a>(q: &str, txns: &'a [ResolvedTransaction]) -> Vec<&'a str> {
        let query = parse_query(q).unwrap();
        txns.iter()
            .filter(|t| query.matches_transaction(t))
            .map(|t| t.description.as_str())
            .collect()
    }

    #[test]
    fn bare_term_matches_account_or_description() {
        let t = txns();
        assert_eq!(matching("coffee", &t), vec!["Coffee Shop"]);
        assert_eq!(matching("rent", &t), vec!["Rent payment"]);
        assert_eq!(matching("checking", &t).len(), 3);
    }

    #[test]
    fn acct_prefix_is_account_only() {
        let t = txns();
        assert_eq!(matching("acct:coffee", &t), vec!["Coffee Shop"]);
        assert!(matching("acct:Shop", &t).is_empty());
    }

    #[test]
    fn desc_prefix() {
        let t = txns();
        assert_eq!(matching("desc:salary", &t), vec!["Salary"]);
        assert_eq!(matching("desc:'coffee shop'", &t), vec!["Coffee Shop"]);
    }

    #[test]
    fn amt_comparisons_magnitude_when_unsigned() {
        let t = txns();
        assert_eq!(matching("amt:>1000", &t), vec!["Rent payment", "Salary"]);
        assert_eq!(matching("amt:4.50", &t), vec!["Coffee Shop"]);
        // Signed: exact sign comparison.
        assert_eq!(matching("amt:<-1500", &t), vec!["Salary"]);
    }

    #[test]
    fn cur_is_whole_symbol() {
        let t = txns();
        assert_eq!(matching("cur:EUR", &t), vec!["Salary"]);
        assert_eq!(matching("cur:E", &t).len(), 0);
        assert_eq!(matching("cur:\\$", &t).len(), 2);
    }

    #[test]
    fn status_terms() {
        let t = txns();
        assert_eq!(matching("status:*", &t), vec!["Coffee Shop"]);
        assert_eq!(matching("status:!", &t), vec!["Salary"]);
        assert_eq!(matching("status:", &t), vec!["Rent payment"]);
    }

    #[test]
    fn date_terms() {
        let t = txns();
        assert_eq!(matching("date:2024-01", &t).len(), 2);
        assert_eq!(matching("date:2024-02", &t), vec!["Salary"]);
        assert_eq!(matching("date:2024-01-10..", &t).len(), 2);
        assert_eq!(matching("date:..2024-01-31", &t).len(), 2);
    }

    #[test]
    fn tag_terms() {
        let t = txns();
        assert_eq!(matching("tag:category", &t), vec!["Coffee Shop"]);
        assert_eq!(matching("tag:category=fun", &t), vec!["Coffee Shop"]);
        assert!(matching("tag:category=boring", &t).is_empty());
    }

    #[test]
    fn retaining_postings_keeps_only_the_matching_side() {
        // Filtering whole transactions would drag the funding posting into an
        // `acct:expenses` report and double the total.
        let journal = hledger_parser::parse(concat!(
            "2024-01-05 Groceries\n",
            "    expenses:food     $50.00\n",
            "    assets:checking\n\n",
            "2024-01-06 Salary\n",
            "    assets:checking  $500.00\n",
            "    income:salary\n",
        ))
        .unwrap();
        let txns = crate::balance::resolve_transactions(&journal).unwrap();

        let q = parse_query("acct:expenses").unwrap();
        let kept = retain_matching_postings(&txns, &q);

        assert_eq!(kept.len(), 1, "the salary transaction has no expense posting");
        assert_eq!(kept[0].postings.len(), 1, "only the expense posting survives");
        assert_eq!(kept[0].postings[0].account.full, "expenses:food");
    }

    #[test]
    fn not_prefix_excludes() {
        let t = txns();
        assert_eq!(
            matching("not:acct:expenses", &t),
            vec!["Coffee Shop", "Rent payment", "Salary"],
            "txn matches when any posting matches; assets postings survive not:expenses"
        );
        // Combined: expense postings that are not rent.
        let query = parse_query("acct:expenses not:rent").unwrap();
        let hits: Vec<&str> = t
            .iter()
            .flat_map(|txn| {
                txn.postings
                    .iter()
                    .filter(|p| query.matches_posting(txn, p))
                    .map(|p| p.account.full.as_str())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(hits, vec!["expenses:food:coffee"]);
    }

    #[test]
    fn same_type_or_different_type_and() {
        let t = txns();
        // Two acct terms OR together.
        assert_eq!(matching("acct:coffee acct:rent", &t).len(), 2);
        // acct AND date.
        assert_eq!(matching("acct:expenses date:2024-01-10..", &t), vec!["Rent payment"]);
    }

    #[test]
    fn invalid_regex_is_error_not_silent() {
        assert!(parse_query("acct:[unclosed").is_err());
        assert!(parse_query("amt:abc").is_err());
    }

    #[test]
    fn colon_in_bare_term_is_account_pattern() {
        let t = txns();
        assert_eq!(matching("expenses:food", &t), vec!["Coffee Shop"]);
    }

    #[test]
    fn empty_query_matches_all() {
        let t = txns();
        assert_eq!(matching("", &t).len(), 3);
        assert_eq!(matching("   ", &t).len(), 3);
    }
}
