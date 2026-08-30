use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use chrono::NaiveDate;
use regex::RegexBuilder;
use rust_decimal::Decimal;
use serde::Serialize;

use hledger_parser::ast::{
    AccountName, AmountStyle, BalanceAssertion, Comment, Posting, PostingAmount, SourceSpan,
    Status, Transaction,
};
use hledger_parser::csv_rules::{CsvRules, IfBlock};

/// Result of converting CSV rows using rules.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportResult {
    pub transactions: Vec<Transaction>,
    pub warnings: Vec<String>,
    pub rows_processed: usize,
}

/// Convert CSV text into transactions using the given rules.
pub fn convert_csv(csv_text: &str, rules: &CsvRules) -> Result<CsvImportResult, String> {
    // The csv crate takes a single byte; `as u8` silently truncated `§` to
    // a different character and split rows on garbage (and a non-ASCII
    // byte in the 0x80..0xFF range corrupts UTF-8 fields).
    if !rules.separator.is_ascii() {
        return Err(format!(
            "separator '{}' is not supported: the field separator must be a single ASCII character",
            rules.separator
        ));
    }
    let delimiter = rules.separator as u8;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .has_headers(false)
        .from_reader(Cursor::new(csv_text));

    let field_index_map: HashMap<String, usize> = rules
        .fields_list
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), i))
        .collect();

    let mut transactions = Vec::new();
    let mut warnings = Vec::new();
    // Surface rules-file warnings (e.g. unknown directives) to the caller.
    warnings.extend(rules.warnings.iter().cloned());
    let mut row_index = 0;
    let mut data_rows = 0;

    for result in reader.records() {
        let record = result.map_err(|e| format!("CSV parse error at row {}: {}", row_index + 1, e))?;
        row_index += 1;

        // Skip header rows
        if row_index <= rules.skip {
            continue;
        }
        data_rows += 1;

        let fields: Vec<String> = record.iter().map(|f| f.to_string()).collect();

        match convert_row(&fields, rules, &field_index_map, row_index) {
            Ok(txn) => transactions.push(txn),
            Err(msg) => warnings.push(format!("Row {}: {}", row_index, msg)),
        }
    }

    if rules.newest_first {
        transactions.reverse();
    }

    Ok(CsvImportResult {
        transactions,
        warnings,
        rows_processed: data_rows,
    })
}

/// A single parsed matcher line from an if-block.
struct MatcherLine<'a> {
    /// This line ANDs with the previous line's group (`&` prefix).
    and_prev: bool,
    /// Negate the match result (`!` prefix; per hledger 1.32 only recognized
    /// when the line is not `&`-prefixed).
    negated: bool,
    /// Match against this named field only (`%fieldname` / `%N` prefix);
    /// None = match against the whole comma-joined record.
    field: Option<String>,
    /// The regex pattern (matched case-insensitively).
    pattern: &'a str,
}

fn parse_matcher_line(line: &str) -> MatcherLine<'_> {
    let mut s = line.trim();
    let and_prev = s.starts_with('&');
    if and_prev {
        s = s[1..].trim_start();
    }
    let mut negated = false;
    // hledger 1.32: '!' negation is only recognized at the start of a
    // non-'&' matcher line; after '&' it is treated as part of the pattern.
    if !and_prev && s.starts_with('!') {
        negated = true;
        s = s[1..].trim_start();
    }
    let mut field = None;
    if s.starts_with('%') {
        let end = s.find(char::is_whitespace).unwrap_or(s.len());
        field = Some(s[1..end].to_lowercase());
        s = s[end..].trim_start();
    }
    MatcherLine { and_prev, negated, field, pattern: s }
}

/// Evaluate an if-block against a row per hledger semantics:
/// consecutive matcher lines are OR alternatives; a `&`-prefixed line ANDs
/// with the preceding line's group. Patterns are case-insensitive regexes.
fn if_block_matches(
    if_block: &IfBlock,
    fields: &[String],
    row_text: &str,
    field_index_map: &HashMap<String, usize>,
) -> bool {
    if if_block.patterns.is_empty() {
        return false;
    }

    let eval_one = |line: &str| -> (bool, bool) {
        let m = parse_matcher_line(line);
        let target: &str = match &m.field {
            None => row_text,
            Some(name) => {
                // %N (1-based column index) or %fieldname
                let idx = if let Ok(n) = name.parse::<usize>() {
                    if n >= 1 { Some(n - 1) } else { None }
                } else {
                    field_index_map.get(name).copied()
                };
                match idx {
                    Some(i) if i < fields.len() => &fields[i],
                    _ => "",
                }
            }
        };
        let matched = RegexBuilder::new(m.pattern)
            .case_insensitive(true)
            .build()
            .map(|re| re.is_match(target))
            .unwrap_or(false);
        (m.and_prev, if m.negated { !matched } else { matched })
    };

    let mut group: Option<bool> = None;
    for line in &if_block.patterns {
        let (and_prev, result) = eval_one(line);
        if and_prev {
            group = Some(group.unwrap_or(true) && result);
        } else {
            if group == Some(true) {
                return true;
            }
            group = Some(result);
        }
    }
    group == Some(true)
}

fn make_posting(
    line: usize,
    account: &str,
    amount: Option<PostingAmount>,
    balance_assertion: Option<BalanceAssertion>,
) -> Posting {
    Posting {
        span: SourceSpan { start: 0, end: 0, line },
        status: Status::Unmarked,
        account: AccountName::new(account),
        amount,
        balance_assertion,
        comment: None,
        tags: vec![],
        is_virtual: false,
        virtual_balanced: false,
        date: None,
        date2: None,
    }
}

fn convert_row(
    fields: &[String],
    rules: &CsvRules,
    field_index_map: &HashMap<String, usize>,
    row_index: usize,
) -> Result<Transaction, String> {
    // Evaluate if-blocks: hledger matches `if PATTERN` against the
    // comma-joined (unquoted) record.
    let row_text = fields.join(",");
    let mut overrides: HashMap<String, String> = HashMap::new();
    for if_block in &rules.if_blocks {
        if if_block_matches(if_block, fields, &row_text, field_index_map) {
            for (key, value) in &if_block.assignments {
                // hledger: later matching if-blocks override earlier ones.
                overrides.insert(key.clone(), value.clone());
            }
        }
    }

    // Resolve field values: overrides, then top-level assignments, then CSV columns.
    let resolve = |name: &str| -> Option<String> {
        if let Some(val) = overrides.get(name) {
            return Some(substitute_fields(val, fields, field_index_map));
        }
        if let Some(val) = rules.field_assignments.get(name) {
            return Some(substitute_fields(val, fields, field_index_map));
        }
        if let Some(&idx) = field_index_map.get(name) {
            if idx < fields.len() {
                let val = fields[idx].trim().to_string();
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
        None
    };
    let resolve_nonempty =
        |name: &str| -> Option<String> { resolve(name).filter(|s| !s.trim().is_empty()) };

    // Parse date
    let date_str = resolve("date").ok_or("No date field")?;
    let date = parse_csv_date(&date_str, rules.date_format.as_deref())
        .map_err(|e| format!("Bad date '{}': {}", date_str, e))?;

    // Parse description
    let description = resolve("description").unwrap_or_default();

    // Status: `*` = cleared, `!` = pending, empty/unset = unmarked (hledger default).
    let status = match resolve_nonempty("status").as_deref() {
        Some("*") => Status::Cleared,
        Some("!") => Status::Pending,
        Some(other) => return Err(format!("Invalid status '{}'", other)),
        None => Status::Unmarked,
    };

    // Code
    let code = resolve_nonempty("code");

    // Parse the primary (first posting) amount:
    // amount1 takes precedence, then amount, then amount-in/amount-out.
    let primary = resolve_primary_amount(rules, &resolve_nonempty)?;
    let amount = primary.quantity;
    // A `currency` rule wins; otherwise a commodity written in the cell
    // (`12 EUR`, `€12`) is kept, as hledger does.
    let (commodity, style) = match (rules.currency.clone(), primary.commodity) {
        (Some(c), _) => {
            let style = amount_style_for(&c);
            (c, style)
        }
        (None, Some((c, style))) => (c, style),
        (None, None) => (String::new(), amount_style_for("")),
    };

    // Comment
    let comment = resolve_nonempty("comment");

    let make_amount = |qty: Decimal| PostingAmount {
        quantity: qty,
        commodity: commodity.clone(),
        style: style.clone(),
        cost: None,
        multiplier: false,
    };
    let make_assertion = |name: &str| -> Result<Option<BalanceAssertion>, String> {
        match resolve_nonempty(name) {
            Some(s) => {
                let qty = parse_amount_str(&s, rules)?;
                Ok(Some(BalanceAssertion {
                    strong: false,
                    inclusive: false,
                    quantity: qty,
                    commodity: commodity.clone(),
                    style: style.clone(),
                }))
            }
            None => Ok(None),
        }
    };

    // Posting 1: account1, primary amount, balance/balance1 assertion.
    let account1 = resolve_nonempty("account1").unwrap_or_else(|| "expenses:unknown".to_string());
    let assertion1 = match make_assertion("balance1")? {
        Some(a) => Some(a),
        None => make_assertion("balance")?,
    };
    let mut postings = vec![make_posting(
        row_index,
        &account1,
        Some(make_amount(amount)),
        assertion1,
    )];

    // Posting 2: account2 (defaulting to income:/expenses:unknown by sign),
    // amount2 if assigned, otherwise inferred (= negation of amount1).
    let amount2 = match resolve_nonempty("amount2") {
        Some(s) => Some(parse_amount_str(&s, rules)?),
        None => None,
    };
    let account2 = resolve_nonempty("account2").unwrap_or_else(|| {
        if amount >= Decimal::ZERO {
            "income:unknown".to_string()
        } else {
            "expenses:unknown".to_string()
        }
    });
    postings.push(make_posting(
        row_index,
        &account2,
        amount2.map(make_amount),
        make_assertion("balance2")?,
    ));

    // Postings 3..=9: generated when accountN or amountN is assigned.
    for n in 3..=9usize {
        let acct_name = format!("account{}", n);
        let amt_name = format!("amount{}", n);
        let acct = resolve_nonempty(&acct_name);
        let amt = match resolve_nonempty(&amt_name) {
            Some(s) => Some(parse_amount_str(&s, rules)?),
            None => None,
        };
        if acct.is_none() && amt.is_none() {
            continue;
        }
        let account = acct.unwrap_or_else(|| "expenses:unknown".to_string());
        let assertion = make_assertion(&format!("balance{}", n))?;
        postings.push(make_posting(row_index, &account, amt.map(make_amount), assertion));
    }

    Ok(Transaction {
        span: SourceSpan { start: 0, end: 0, line: row_index },
        date,
        secondary_date: None,
        status,
        code,
        description,
        comment: comment.map(|c| Comment { text: c }),
        tags: vec![],
        postings,
    })
}

fn amount_style_for(commodity: &str) -> AmountStyle {
    hledger_parser::writer::default_style_for(commodity)
}

/// An amount read from a CSV cell: the number, and the commodity and style
/// the cell carried, if any (`12 EUR`, `€12`).
#[derive(Debug, Clone)]
struct CsvAmount {
    quantity: Decimal,
    commodity: Option<(String, AmountStyle)>,
}

impl CsvAmount {
    fn map(self, f: impl FnOnce(Decimal) -> Decimal) -> Self {
        Self {
            quantity: f(self.quantity),
            commodity: self.commodity,
        }
    }
}

fn resolve_primary_amount(
    rules: &CsvRules,
    resolve_nonempty: &dyn Fn(&str) -> Option<String>,
) -> Result<CsvAmount, String> {
    // Modern style: amount1 is the first posting's amount.
    if let Some(amt_str) = resolve_nonempty("amount1") {
        return parse_amount_cell(&amt_str, rules);
    }

    // Old style: "amount" is the (first posting's) transaction amount.
    if let Some(amt_str) = resolve_nonempty("amount") {
        return parse_amount_cell(&amt_str, rules);
    }

    // amount-in / amount-out pair
    if let Some(in_str) = resolve_nonempty("amount-in") {
        return Ok(parse_amount_cell(&in_str, rules)?.map(|q| q.abs()));
    }
    if let Some(out_str) = resolve_nonempty("amount-out") {
        return Ok(parse_amount_cell(&out_str, rules)?.map(|q| -q.abs()));
    }

    Err("No amount field found".to_string())
}

fn parse_amount_str(s: &str, rules: &CsvRules) -> Result<Decimal, String> {
    parse_amount_cell(s, rules).map(|a| a.quantity)
}

/// Read an amount cell the way hledger's CSV reader does: with the journal
/// amount parser, under the rules' `decimal-mark`. So `1.234,56` is
/// 1234.56 (hledger infers the rightmost separator), `12 EUR` and `€12`
/// carry their commodity, and `(12.34)` is negative.
fn parse_amount_cell(s: &str, rules: &CsvRules) -> Result<CsvAmount, String> {
    let trimmed = s.trim();

    // Parenthesized amounts are negative: (12.34) => -12.34
    let (inner, parenthesized) = if trimmed.starts_with('(') && trimmed.ends_with(')') && trimmed.len() >= 2 {
        (trimmed[1..trimmed.len() - 1].trim(), true)
    } else {
        (trimmed, false)
    };
    if inner.is_empty() {
        return Err("Empty amount".to_string());
    }

    let ctx = hledger_parser::AmountContext {
        decimal_mark: rules.decimal_mark,
        ..Default::default()
    };
    let amt = hledger_parser::parse_amount_ctx(inner, &ctx)
        .map_err(|e| format!("Invalid amount '{}': {}", s, e))?;

    let quantity = if parenthesized { -amt.quantity.abs() } else { amt.quantity };
    let commodity = if amt.commodity.is_empty() {
        None
    } else {
        Some((amt.commodity, amt.style))
    };
    Ok(CsvAmount { quantity, commodity })
}

fn parse_csv_date(date_str: &str, date_format: Option<&str>) -> Result<NaiveDate, String> {
    let s = date_str.trim();
    match date_format {
        // An explicit date-format must match; no silent fallbacks (hledger
        // treats a mismatch as an error for that row).
        Some(fmt) => NaiveDate::parse_from_str(s, fmt).map_err(|e| e.to_string()),
        // No date-format: accept hledger's simple date forms.
        None => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .or_else(|_| NaiveDate::parse_from_str(s, "%Y/%m/%d"))
            .or_else(|_| NaiveDate::parse_from_str(s, "%Y.%m.%d"))
            .map_err(|e| e.to_string()),
    }
}

/// Substitute %fieldname and %N references in a value string.
fn substitute_fields(
    template: &str,
    fields: &[String],
    field_index_map: &HashMap<String, usize>,
) -> String {
    if !template.contains('%') {
        return template.to_string();
    }

    let mut result = String::new();
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            if chars[i + 1] == '%' {
                result.push('%');
                i += 2;
                continue;
            }

            // Try numeric reference %1, %2, etc.
            if chars[i + 1].is_ascii_digit() {
                let mut num_str = String::new();
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    num_str.push(chars[j]);
                    j += 1;
                }
                if let Ok(n) = num_str.parse::<usize>() {
                    if n >= 1 && n <= fields.len() {
                        result.push_str(fields[n - 1].trim());
                    }
                }
                i = j;
                continue;
            }

            // Try field name reference %fieldname
            let mut name = String::new();
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '-') {
                name.push(chars[j]);
                j += 1;
            }
            let lower_name = name.to_lowercase();
            if let Some(&idx) = field_index_map.get(&lower_name) {
                if idx < fields.len() {
                    result.push_str(fields[idx].trim());
                }
            }
            i = j;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Duplicate detection
// ---------------------------------------------------------------------------

/// Compute a stable fingerprint for a transaction: exact date, first posting
/// amount (quantity + commodity), and case/whitespace-normalized description.
pub fn transaction_fingerprint(txn: &Transaction) -> String {
    let amount = txn
        .postings
        .iter()
        .find_map(|p| p.amount.as_ref())
        .map(|a| format!("{} {}", a.quantity.normalize(), a.commodity))
        .unwrap_or_default();
    format!(
        "{}|{}|{}",
        txn.date,
        amount,
        normalize_description(&txn.description)
    )
}

fn normalize_description(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Given candidate (imported) transactions and the existing ledger's
/// transactions, return a bool per candidate: true = probable duplicate of an
/// existing transaction (same date, first-posting amount, and normalized
/// description).
pub fn mark_probable_duplicates(candidates: &[Transaction], existing: &[Transaction]) -> Vec<bool> {
    let existing_fps: HashSet<String> = existing.iter().map(transaction_fingerprint).collect();
    candidates
        .iter()
        .map(|t| existing_fps.contains(&transaction_fingerprint(t)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hledger_parser::csv_rules::parse_csv_rules;

    #[test]
    fn amounts_are_read_like_hledger_with_commodities_from_the_cell() {
        // hledger print on the same CSV: 1.234,56 / 12 EUR / €12 / $1,000.50.
        let rules = parse_csv_rules("fields date,description,amount\n").unwrap();
        let csv = "2024-01-01,foo,\"1.234,56\"\n2024-01-02,bar,12 EUR\n2024-01-03,baz,€12\n2024-01-04,qux,\"$1,000.50\"\n2024-01-05,neg,(12.34)\n";
        let result = convert_csv(csv, &rules).unwrap();
        let first = |t: &Transaction| {
            let a = t.postings[0].amount.as_ref().unwrap();
            (a.quantity, a.commodity.clone())
        };
        let got: Vec<_> = result.transactions.iter().map(first).collect();
        assert_eq!(
            got,
            vec![
                (Decimal::new(123456, 2), String::new()),
                (Decimal::new(12, 0), "EUR".to_string()),
                (Decimal::new(12, 0), "€".to_string()),
                (Decimal::new(100050, 2), "$".to_string()),
                (Decimal::new(-1234, 2), String::new()),
            ]
        );
        // The commodity's style came from the cell too.
        let euro = result.transactions[2].postings[0].amount.as_ref().unwrap();
        assert_eq!(euro.style.commodity_side, hledger_parser::ast::Side::Left);
        // The inferred second posting carries the same commodity.
        assert!(result.transactions[1].postings[1].amount.is_none());

        // A `currency` rule still wins over what the cell says.
        let rules = parse_csv_rules("fields date,description,amount\ncurrency USD\n").unwrap();
        let result = convert_csv("2024-01-02,bar,12 EUR\n", &rules).unwrap();
        assert_eq!(result.transactions[0].postings[0].amount.as_ref().unwrap().commodity, "USD");

        // A decimal-mark rule is honoured.
        let rules = parse_csv_rules("fields date,description,amount\ndecimal-mark ,\n").unwrap();
        let result = convert_csv("2024-01-02,bar,\"1.234\"\n", &rules).unwrap();
        assert_eq!(result.transactions[0].postings[0].amount.as_ref().unwrap().quantity, Decimal::new(1234, 0));
    }

    #[test]
    fn non_ascii_separator_is_an_error_not_a_truncation() {
        let rules = parse_csv_rules("separator §\nfields date,description,amount\n").unwrap();
        let err = convert_csv("2024-01-01§x§1\n", &rules).unwrap_err();
        assert!(err.contains("separator"), "{}", err);
    }

    #[test]
    fn convert_simple_csv() {
        let rules_text = r#"
skip 1
fields date, description, amount
date-format %m/%d/%Y
currency $
account1 assets:checking
account2 expenses:unknown
"#;
        let csv_text = r#"Date,Description,Amount
03/15/2026,WHOLE FOODS,-87.42
03/14/2026,EMPLOYER SALARY,3200.00
"#;
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();

        assert_eq!(result.transactions.len(), 2);
        assert_eq!(result.rows_processed, 2);
        assert!(result.warnings.is_empty());

        let t0 = &result.transactions[0];
        assert_eq!(t0.date, NaiveDate::from_ymd_opt(2026, 3, 15).unwrap());
        assert_eq!(t0.description, "WHOLE FOODS");
        assert_eq!(t0.postings[0].account.full, "assets:checking");
        assert_eq!(t0.postings[0].amount.as_ref().unwrap().quantity, Decimal::from_str_exact("-87.42").unwrap());
        assert_eq!(t0.postings[1].account.full, "expenses:unknown");
    }

    #[test]
    fn convert_headerless_csv_without_skip() {
        // hledger's default skip is 0: a headerless CSV must keep its first row.
        let rules_text = "fields date, description, amount\naccount1 assets:a\n";
        let csv_text = "2026-01-01,foo,10\n2026-01-02,bar,20\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions.len(), 2);
        assert_eq!(result.transactions[0].description, "foo");
    }

    #[test]
    fn convert_with_if_blocks() {
        let rules_text = r#"
skip 1
fields date, description, amount
date-format %Y-%m-%d
account1 assets:checking

if WHOLE FOODS
  account2 expenses:groceries

if SALARY
  account2 income:salary
"#;
        let csv_text = "date,desc,amt\n2026-01-15,WHOLE FOODS,-50.00\n2026-01-16,EMPLOYER SALARY,3000.00\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();

        assert_eq!(result.transactions[0].postings[1].account.full, "expenses:groceries");
        assert_eq!(result.transactions[1].postings[1].account.full, "income:salary");
    }

    #[test]
    fn later_if_block_wins() {
        // Verified against hledger 1.32.3: later matching if-blocks override
        // earlier ones.
        let rules_text = "skip 1\nfields date, description, amount\naccount1 assets:a\n\nif FOODS\n account2 expenses:first\n\nif WHOLE\n account2 expenses:second\n";
        let csv_text = "date,desc,amt\n2026-01-01,WHOLE FOODS,10\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions[0].postings[1].account.full, "expenses:second");
    }

    #[test]
    fn field_matcher_matches_only_that_field() {
        // `if %description ^SHOP` must not match "COFFEE SHOP" (anchored to
        // the field), and `if ^SHOP` must not match the record (which starts
        // with the date). Verified against hledger 1.32.3.
        let rules_text = "skip 1\nfields date, description, amount\naccount1 assets:a\n\nif %description ^SHOP\n account2 expenses:matched\n";
        let csv_text = "date,desc,amt\n2026-01-01,COFFEE SHOP,-4\n2026-01-02,SHOP,-5\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions[0].postings[1].account.full, "expenses:unknown");
        assert_eq!(result.transactions[1].postings[1].account.full, "expenses:matched");

        let rules_text2 = "skip 1\nfields date, description, amount\naccount1 assets:a\n\nif ^SHOP\n account2 expenses:matched\n";
        let rules2 = parse_csv_rules(rules_text2).unwrap();
        let result2 = convert_csv(csv_text, &rules2).unwrap();
        assert_eq!(result2.transactions[1].postings[1].account.full, "expenses:unknown");
    }

    #[test]
    fn and_combinator() {
        let rules_text = "skip 1\nfields date, description, amount\naccount1 assets:a\n\nif COFFEE\n& SHOP\n account2 expenses:both\n";
        let csv_text = "date,desc,amt\n2026-01-01,COFFEE SHOP,-4\n2026-01-02,SHOP,-5\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions[0].postings[1].account.full, "expenses:both");
        assert_eq!(result.transactions[1].postings[1].account.full, "expenses:unknown");
    }

    #[test]
    fn negation_matcher() {
        // `if ! COFFEE` and `if !COFFEE` match rows NOT containing COFFEE.
        for pat in ["if ! COFFEE", "if !COFFEE"] {
            let rules_text = format!(
                "skip 1\nfields date, description, amount\naccount1 assets:a\n\n{}\n account2 expenses:notcoffee\n",
                pat
            );
            let csv_text = "date,desc,amt\n2026-01-01,COFFEE SHOP,-4\n2026-01-02,SHOP,-5\n";
            let rules = parse_csv_rules(&rules_text).unwrap();
            let result = convert_csv(csv_text, &rules).unwrap();
            assert_eq!(result.transactions[0].postings[1].account.full, "expenses:unknown");
            assert_eq!(result.transactions[1].postings[1].account.full, "expenses:notcoffee");
        }
    }

    #[test]
    fn negated_field_matcher() {
        let rules_text = "skip 1\nfields date, description, amount\naccount1 assets:a\n\nif ! %description COFFEE\n account2 expenses:n2\n";
        let csv_text = "date,desc,amt\n2026-01-01,COFFEE SHOP,-4\n2026-01-02,SHOP,-5\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions[0].postings[1].account.full, "expenses:unknown");
        assert_eq!(result.transactions[1].postings[1].account.full, "expenses:n2");
    }

    #[test]
    fn or_and_grouping() {
        // A \n B \n & X  =>  A OR (B AND X). Verified against hledger 1.32.3.
        let rules_text = "skip 1\nfields date, description, amount\naccount1 assets:a\n\nif A\nB\n& X\n account2 expenses:g\n";
        let csv_text = "date,desc,amt\n2026-01-01,AX,-1\n2026-01-02,BX,-2\n2026-01-03,BY,-3\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions[0].postings[1].account.full, "expenses:g");
        assert_eq!(result.transactions[1].postings[1].account.full, "expenses:g");
        assert_eq!(result.transactions[2].postings[1].account.full, "expenses:unknown");
    }

    #[test]
    fn negation_after_ampersand_is_literal() {
        // hledger 1.32.3 does not negate after '&': "& !COFFEE" looks for the
        // literal regex "!COFFEE", so neither row matches.
        let rules_text = "skip 1\nfields date, description, amount\naccount1 assets:a\n\nif SHOP\n& !COFFEE\n account2 expenses:x\n";
        let csv_text = "date,desc,amt\n2026-01-01,COFFEE SHOP,-4\n2026-01-02,SHOP,-5\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions[0].postings[1].account.full, "expenses:unknown");
        assert_eq!(result.transactions[1].postings[1].account.full, "expenses:unknown");
    }

    #[test]
    fn matchers_are_case_insensitive() {
        let rules_text = "skip 1\nfields date, description, amount\naccount1 assets:a\n\nif shop\n account2 expenses:ci\n";
        let csv_text = "date,desc,amt\n2026-01-01,COFFEE SHOP,-4\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions[0].postings[1].account.full, "expenses:ci");
    }

    #[test]
    fn convert_newest_first() {
        let rules_text = "newest-first\nskip 1\nfields date, description, amount\n";
        let csv_text = "d,d,a\n2026-03-15,B,-10\n2026-03-14,A,-20\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();

        // Should be reversed: A (older) first
        assert_eq!(result.transactions[0].description, "A");
        assert_eq!(result.transactions[1].description, "B");
    }

    #[test]
    fn convert_european_decimal() {
        let rules_text = "decimal-mark ,\nskip 1\nfields date, description, amount\nseparator ;\n";
        let csv_text = "d;d;a\n2026-01-01;Test;1.234,56\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();

        assert_eq!(
            result.transactions[0].postings[0].amount.as_ref().unwrap().quantity,
            Decimal::from_str_exact("1234.56").unwrap()
        );
    }

    #[test]
    fn amount_in_out_fields() {
        let rules_text = r#"
skip 1
fields date, description, amount-in, amount-out
account1 assets:checking
"#;
        let csv_text = "d,d,in,out\n2026-01-01,Deposit,500.00,\n2026-01-02,Payment,,200.00\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();

        assert_eq!(
            result.transactions[0].postings[0].amount.as_ref().unwrap().quantity,
            Decimal::from_str_exact("500.00").unwrap()
        );
        assert_eq!(
            result.transactions[1].postings[0].amount.as_ref().unwrap().quantity,
            Decimal::from_str_exact("-200.00").unwrap()
        );
    }

    #[test]
    fn parenthesized_amounts_are_negative() {
        // Verified against hledger 1.32.3: (12.34) => -12.34.
        let rules_text = "skip 1\nfields date, description, amount\naccount1 assets:a\n";
        let csv_text = "date,desc,amt\n2026-01-01,thing,(12.34)\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(
            result.transactions[0].postings[0].amount.as_ref().unwrap().quantity,
            Decimal::from_str_exact("-12.34").unwrap()
        );
        // The inferred second posting goes to expenses:unknown for a negative.
        assert_eq!(result.transactions[0].postings[1].account.full, "expenses:unknown");
    }

    #[test]
    fn parenthesized_with_currency_symbol() {
        let rules_text = "skip 1\nfields date, description, amount\naccount1 assets:a\n";
        let csv_text = "date,desc,amt\n2026-01-01,thing,\"($1,234.56)\"\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(
            result.transactions[0].postings[0].amount.as_ref().unwrap().quantity,
            Decimal::from_str_exact("-1234.56").unwrap()
        );
    }

    #[test]
    fn amount1_field_assignment() {
        // Modern rules style: amount1 %amt must produce transactions.
        let rules_text = "skip 1\nfields date, description, amt\namount1 %amt\naccount1 assets:a\naccount2 expenses:x\n";
        let csv_text = "date,desc,amt\n2026-01-01,foo,-5.00\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions.len(), 1);
        let t = &result.transactions[0];
        assert_eq!(t.postings[0].amount.as_ref().unwrap().quantity, Decimal::from_str_exact("-5.00").unwrap());
        // amount2 unassigned: second posting's amount is inferred (None = negation)
        assert!(t.postings[1].amount.is_none());
        assert_eq!(t.postings[1].account.full, "expenses:x");
    }

    #[test]
    fn amount1_and_amount2() {
        let rules_text = "skip 1\nfields date, description, amt\namount1 %amt\namount2 5.00\naccount1 assets:a\naccount2 expenses:x\n";
        let csv_text = "date,desc,amt\n2026-01-01,foo,-5.00\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        let t = &result.transactions[0];
        assert_eq!(t.postings[0].amount.as_ref().unwrap().quantity, Decimal::from_str_exact("-5.00").unwrap());
        assert_eq!(t.postings[1].amount.as_ref().unwrap().quantity, Decimal::from_str_exact("5.00").unwrap());
    }

    #[test]
    fn amount2_without_account2_uses_unknown() {
        // Verified against hledger 1.32.3: second posting gets
        // expenses:unknown (first posting is negative).
        let rules_text = "skip 1\nfields date, description, amt\namount1 %amt\namount2 5.00\naccount1 assets:a\n";
        let csv_text = "date,desc,amt\n2026-01-01,foo,-5.00\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        let t = &result.transactions[0];
        assert_eq!(t.postings[1].account.full, "expenses:unknown");
        assert_eq!(t.postings[1].amount.as_ref().unwrap().quantity, Decimal::from_str_exact("5.00").unwrap());
    }

    #[test]
    fn three_posting_row() {
        // Verified against hledger 1.32.3 (t4d): amount1/2/3 + account1/2/3.
        let rules_text = "skip 1\nfields date, description, amt\namount1 %amt\namount2 2.00\namount3 3.00\naccount1 assets:a\naccount2 expenses:x\naccount3 expenses:y\n";
        let csv_text = "date,desc,amt\n2026-01-01,foo,-5.00\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        let t = &result.transactions[0];
        assert_eq!(t.postings.len(), 3);
        assert_eq!(t.postings[2].account.full, "expenses:y");
        assert_eq!(t.postings[2].amount.as_ref().unwrap().quantity, Decimal::from_str_exact("3.00").unwrap());
    }

    #[test]
    fn amount1_in_if_block() {
        let rules_text = "skip 1\nfields date, description, amt\naccount1 assets:a\n\nif foo\n  amount1 %amt\n";
        let csv_text = "date,desc,amt\n2026-01-01,foo,-7.50\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(
            result.transactions[0].postings[0].amount.as_ref().unwrap().quantity,
            Decimal::from_str_exact("-7.50").unwrap()
        );
    }

    #[test]
    fn default_status_is_unmarked() {
        // Verified against hledger 1.32.3: no status rule = unmarked.
        let rules_text = "skip 1\nfields date, description, amount\naccount1 assets:a\n";
        let csv_text = "date,desc,amt\n2026-01-01,foo,-4\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions[0].status, Status::Unmarked);
        assert_eq!(result.transactions[0].code, None);
    }

    #[test]
    fn status_and_code_rules() {
        let rules_text = "skip 1\nfields date, description, amount\naccount1 assets:a\nstatus *\ncode X123\n";
        let csv_text = "date,desc,amt\n2026-01-01,foo,-4\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions[0].status, Status::Cleared);
        assert_eq!(result.transactions[0].code.as_deref(), Some("X123"));

        let rules_text2 = "skip 1\nfields date, description, amount\naccount1 assets:a\nstatus !\n";
        let rules2 = parse_csv_rules(rules_text2).unwrap();
        let result2 = convert_csv(csv_text, &rules2).unwrap();
        assert_eq!(result2.transactions[0].status, Status::Pending);
    }

    #[test]
    fn balance_rule_emits_assertion() {
        // Verified against hledger 1.32.3: balance1 %bal => "= 100.00" on the
        // first posting; unnumbered balance behaves the same.
        for balname in ["balance1", "balance"] {
            let rules_text = format!(
                "skip 1\nfields date, description, amount, bal\naccount1 assets:a\n{} %bal\n",
                balname
            );
            let csv_text = "date,desc,amt,bal\n2026-01-01,foo,-4,100.00\n";
            let rules = parse_csv_rules(&rules_text).unwrap();
            let result = convert_csv(csv_text, &rules).unwrap();
            let p0 = &result.transactions[0].postings[0];
            let a = p0.balance_assertion.as_ref().expect("assertion on posting 1");
            assert_eq!(a.quantity, Decimal::from_str_exact("100.00").unwrap());
            assert!(!a.strong);
            assert!(result.transactions[0].postings[1].balance_assertion.is_none());
        }
    }

    #[test]
    fn balance2_rule_on_second_posting() {
        let rules_text = "skip 1\nfields date, description, amount, bal\naccount1 assets:a\naccount2 expenses:x\nbalance2 50\n";
        let csv_text = "date,desc,amt,bal\n2026-01-01,foo,-4,100.00\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        let t = &result.transactions[0];
        assert!(t.postings[0].balance_assertion.is_none());
        let a = t.postings[1].balance_assertion.as_ref().expect("assertion on posting 2");
        assert_eq!(a.quantity, Decimal::from_str_exact("50").unwrap());
    }

    #[test]
    fn date_format_mismatch_is_row_error() {
        // With date-format set, a non-matching date must be a row error, not
        // silently parsed by a fallback format.
        let rules_text = "skip 1\nfields date, description, amount\ndate-format %m/%d/%Y\naccount1 assets:a\n";
        let csv_text = "date,desc,amt\n01/02/2026,foo,-4\n2026-01-03,bar,-5\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions.len(), 1);
        assert_eq!(result.transactions[0].date, NaiveDate::from_ymd_opt(2026, 1, 2).unwrap());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("2026-01-03"));
    }

    #[test]
    fn simple_date_fallbacks_without_date_format() {
        let rules_text = "fields date, description, amount\naccount1 assets:a\n";
        let csv_text = "2026-01-01,a,-1\n2026/01/02,b,-2\n2026.01.03,c,-3\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert_eq!(result.transactions.len(), 3);
        assert_eq!(result.transactions[2].date, NaiveDate::from_ymd_opt(2026, 1, 3).unwrap());
    }

    #[test]
    fn rules_warnings_propagate_to_result() {
        let rules_text = "skipfoo 2\nskip 1\nfields date, description, amount\naccount1 assets:a\n";
        let csv_text = "date,desc,amt\n2026-01-01,foo,-4\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let result = convert_csv(csv_text, &rules).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("skipfoo")));
        assert_eq!(result.transactions.len(), 1);
    }

    #[test]
    fn fingerprint_normalizes_description() {
        let rules_text = "fields date, description, amount\naccount1 assets:a\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let a = convert_csv("2026-01-01,  Whole   FOODS ,-5.00\n", &rules).unwrap().transactions;
        let b = convert_csv("2026-01-01,whole foods,-5.0\n", &rules).unwrap().transactions;
        assert_eq!(transaction_fingerprint(&a[0]), transaction_fingerprint(&b[0]));
    }

    #[test]
    fn fingerprint_distinguishes_date_and_amount() {
        let rules_text = "fields date, description, amount\naccount1 assets:a\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let a = convert_csv("2026-01-01,foo,-5.00\n", &rules).unwrap().transactions;
        let b = convert_csv("2026-01-02,foo,-5.00\n", &rules).unwrap().transactions;
        let c = convert_csv("2026-01-01,foo,-6.00\n", &rules).unwrap().transactions;
        assert_ne!(transaction_fingerprint(&a[0]), transaction_fingerprint(&b[0]));
        assert_ne!(transaction_fingerprint(&a[0]), transaction_fingerprint(&c[0]));
    }

    #[test]
    fn mark_duplicates_against_existing() {
        let rules_text = "fields date, description, amount\naccount1 assets:a\n";
        let rules = parse_csv_rules(rules_text).unwrap();
        let existing = convert_csv("2026-01-01,coffee,-4.00\n2026-01-02,rent,-900\n", &rules)
            .unwrap()
            .transactions;
        let candidates = convert_csv("2026-01-01,COFFEE,-4.0\n2026-01-03,new thing,-1\n", &rules)
            .unwrap()
            .transactions;
        let flags = mark_probable_duplicates(&candidates, &existing);
        assert_eq!(flags, vec![true, false]);
    }
}
