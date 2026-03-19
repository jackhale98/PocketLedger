use std::collections::HashMap;
use std::io::Cursor;

use chrono::NaiveDate;
use regex::RegexBuilder;
use rust_decimal::Decimal;
use serde::Serialize;

use hledger_parser::ast::{
    AccountName, AmountStyle, Comment, Posting, PostingAmount, Side, SourceSpan, Status, Transaction,
};
use hledger_parser::csv_rules::CsvRules;

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
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(rules.separator as u8)
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

fn convert_row(
    fields: &[String],
    rules: &CsvRules,
    field_index_map: &HashMap<String, usize>,
    row_index: usize,
) -> Result<Transaction, String> {
    // Evaluate if-blocks: join all fields for matching
    let row_text = fields.join(",");
    let mut overrides: HashMap<String, String> = HashMap::new();
    for if_block in &rules.if_blocks {
        let matched = if_block.patterns.iter().any(|pattern| {
            RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .map(|re| re.is_match(&row_text))
                .unwrap_or(false)
        });
        if matched {
            for (key, value) in &if_block.assignments {
                overrides.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }

    // Resolve field values
    let resolve = |name: &str| -> Option<String> {
        // Check overrides first, then top-level assignments, then fields list
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

    // Parse date
    let date_str = resolve("date").ok_or("No date field")?;
    let date = parse_csv_date(&date_str, rules.date_format.as_deref())
        .map_err(|e| format!("Bad date '{}': {}", date_str, e))?;

    // Parse description
    let description = resolve("description").unwrap_or_default();

    // Parse amount
    let (amount, commodity) = resolve_amount(fields, rules, field_index_map, &overrides)?;

    // Resolve accounts
    let account1 = resolve("account1").unwrap_or_else(|| "expenses:unknown".to_string());
    let account2 = resolve("account2").unwrap_or_else(|| {
        if amount >= Decimal::ZERO { "income:unknown".to_string() } else { "expenses:unknown".to_string() }
    });

    // Comment
    let comment = resolve("comment");

    // Build transaction
    let is_symbol = commodity.len() == 1 && "$\u{20AC}\u{00A3}\u{00A5}\u{20B9}\u{20BD}\u{20BF}".contains(&commodity);
    let style = if is_symbol {
        AmountStyle { commodity_side: Side::Left, commodity_spaced: false, decimal_mark: '.', precision: 2 }
    } else if commodity.is_empty() {
        AmountStyle::default()
    } else {
        AmountStyle { commodity_side: Side::Right, commodity_spaced: true, decimal_mark: '.', precision: 2 }
    };

    let posting1 = Posting {
        span: SourceSpan { start: 0, end: 0, line: row_index },
        status: Status::Unmarked,
        account: AccountName::new(&account1),
        amount: Some(PostingAmount { quantity: amount, commodity: commodity.clone(), style, cost: None }),
        balance_assertion: None,
        comment: None,
        tags: vec![],
        is_virtual: false,
        virtual_balanced: false,
    };

    let posting2 = Posting {
        span: SourceSpan { start: 0, end: 0, line: row_index },
        status: Status::Unmarked,
        account: AccountName::new(&account2),
        amount: None, // Inferred
        balance_assertion: None,
        comment: None,
        tags: vec![],
        is_virtual: false,
        virtual_balanced: false,
    };

    Ok(Transaction {
        span: SourceSpan { start: 0, end: 0, line: row_index },
        date,
        secondary_date: None,
        status: Status::Cleared,
        code: None,
        description,
        comment: comment.map(|c| Comment { text: c }),
        tags: vec![],
        postings: vec![posting1, posting2],
    })
}

fn resolve_amount(
    fields: &[String],
    rules: &CsvRules,
    field_index_map: &HashMap<String, usize>,
    overrides: &HashMap<String, String>,
) -> Result<(Decimal, String), String> {
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

    let currency = rules.currency.clone().unwrap_or_default();

    // Try "amount" field first
    if let Some(amt_str) = resolve("amount") {
        let qty = parse_amount_str(&amt_str, rules)?;
        return Ok((qty, currency));
    }

    // Try amount-in / amount-out pair
    let amount_in = resolve("amount-in").and_then(|s| {
        let s = s.trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    });
    let amount_out = resolve("amount-out").and_then(|s| {
        let s = s.trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    });

    if let Some(in_str) = amount_in {
        let qty = parse_amount_str(&in_str, rules)?.abs();
        return Ok((qty, currency));
    }
    if let Some(out_str) = amount_out {
        let qty = -(parse_amount_str(&out_str, rules)?.abs());
        return Ok((qty, currency));
    }

    Err("No amount field found".to_string())
}

fn parse_amount_str(s: &str, rules: &CsvRules) -> Result<Decimal, String> {
    // Strip currency symbols and whitespace
    let cleaned: String = s.chars().filter(|c| {
        c.is_ascii_digit() || *c == '-' || *c == '+' || *c == '.' || *c == ','
    }).collect();

    if cleaned.is_empty() {
        return Err("Empty amount".to_string());
    }

    // Handle decimal mark
    let normalized = match rules.decimal_mark {
        Some(',') => {
            // European: periods are thousands separators, comma is decimal
            cleaned.replace('.', "").replace(',', ".")
        }
        _ => {
            // Standard: commas are thousands separators, period is decimal
            cleaned.replace(',', "")
        }
    };

    Decimal::from_str_exact(&normalized)
        .map_err(|e| format!("Invalid amount '{}': {}", s, e))
}

fn parse_csv_date(date_str: &str, date_format: Option<&str>) -> Result<NaiveDate, String> {
    let fmt = date_format.unwrap_or("%Y-%m-%d");
    // Also try common variants
    NaiveDate::parse_from_str(date_str.trim(), fmt)
        .or_else(|_| NaiveDate::parse_from_str(date_str.trim(), "%Y/%m/%d"))
        .or_else(|_| NaiveDate::parse_from_str(date_str.trim(), "%Y-%m-%d"))
        .map_err(|e| e.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use hledger_parser::csv_rules::parse_csv_rules;

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
}
