use std::collections::HashMap;

use crate::error::ParseError;

/// A parsed CSV rules file.
#[derive(Debug, Clone, PartialEq)]
pub struct CsvRules {
    /// Number of header lines to skip (default 1).
    pub skip: usize,
    /// Field separator character (default ',').
    pub separator: char,
    /// Date format string (strftime-style, e.g. "%m/%d/%Y"). None = "%Y-%m-%d".
    pub date_format: Option<String>,
    /// Default currency/commodity to prepend to amounts.
    pub currency: Option<String>,
    /// Decimal mark character ('.' or ','). None = '.'.
    pub decimal_mark: Option<char>,
    /// Whether CSV rows are newest-first (default false = oldest-first).
    pub newest_first: bool,
    /// Field names in CSV column order (from the `fields` directive).
    pub fields_list: Vec<String>,
    /// Top-level field assignments (e.g. account1 -> "assets:checking").
    pub field_assignments: HashMap<String, String>,
    /// Conditional blocks, evaluated in order.
    pub if_blocks: Vec<IfBlock>,
}

/// A conditional block: if any pattern matches, apply the assignments.
#[derive(Debug, Clone, PartialEq)]
pub struct IfBlock {
    /// Regex patterns (any must match). Matched against the full CSV row.
    pub patterns: Vec<String>,
    /// Field assignments to apply when matched.
    pub assignments: HashMap<String, String>,
}

impl Default for CsvRules {
    fn default() -> Self {
        Self {
            skip: 1,
            separator: ',',
            date_format: None,
            currency: None,
            decimal_mark: None,
            newest_first: false,
            fields_list: Vec::new(),
            field_assignments: HashMap::new(),
            if_blocks: Vec::new(),
        }
    }
}

/// Parse a .csv.rules file from its text content.
pub fn parse_csv_rules(input: &str) -> Result<CsvRules, ParseError> {
    let mut rules = CsvRules::default();
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Skip blank lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            i += 1;
            continue;
        }

        // Directives
        if let Some(rest) = trimmed.strip_prefix("skip") {
            let rest = rest.trim();
            if rest.is_empty() {
                rules.skip = 1;
            } else {
                rules.skip = rest.parse::<usize>().unwrap_or(1);
            }
            i += 1;
        } else if let Some(rest) = trimmed.strip_prefix("separator") {
            let rest = rest.trim();
            rules.separator = match rest {
                "\\t" | "TAB" | "tab" => '\t',
                s if s.len() == 1 => s.chars().next().unwrap(),
                _ => ',',
            };
            i += 1;
        } else if let Some(rest) = trimmed.strip_prefix("date-format") {
            rules.date_format = Some(rest.trim().to_string());
            i += 1;
        } else if let Some(rest) = trimmed.strip_prefix("decimal-mark") {
            let rest = rest.trim();
            if let Some(c) = rest.chars().next() {
                rules.decimal_mark = Some(c);
            }
            i += 1;
        } else if trimmed.starts_with("newest-first") {
            rules.newest_first = true;
            i += 1;
        } else if let Some(rest) = trimmed.strip_prefix("fields") {
            let fields: Vec<String> = rest
                .split(',')
                .map(|f| f.trim().to_lowercase().to_string())
                .filter(|f| !f.is_empty())
                .collect();
            rules.fields_list = fields;
            i += 1;
        } else if let Some(rest) = trimmed.strip_prefix("currency") {
            rules.currency = Some(rest.trim().to_string());
            i += 1;
        } else if trimmed.starts_with("if") {
            // Parse if block
            let (if_block, next_i) = parse_if_block(&lines, i)?;
            rules.if_blocks.push(if_block);
            i = next_i;
        } else if let Some((name, value)) = parse_field_assignment(trimmed) {
            rules.field_assignments.insert(name, value);
            i += 1;
        } else {
            // Unknown directive - skip
            i += 1;
        }
    }

    Ok(rules)
}

/// Known field names for assignments.
const FIELD_NAMES: &[&str] = &[
    "account1", "account2", "account3", "account4",
    "amount", "amount-in", "amount-out",
    "date", "date2", "description", "comment", "status", "code",
    "balance", "balance1", "balance2",
];

fn parse_field_assignment(line: &str) -> Option<(String, String)> {
    for &name in FIELD_NAMES {
        if let Some(rest) = line.strip_prefix(name) {
            if rest.starts_with(' ') || rest.starts_with('\t') {
                return Some((name.to_string(), rest.trim().to_string()));
            }
        }
    }
    None
}

fn parse_if_block(lines: &[&str], start: usize) -> Result<(IfBlock, usize), ParseError> {
    let mut patterns = Vec::new();
    let mut assignments = HashMap::new();
    let mut i = start;

    // The first line is "if" optionally followed by a pattern
    let first_line = lines[i].trim();
    let after_if = first_line.strip_prefix("if").unwrap().trim();
    if !after_if.is_empty() {
        patterns.push(after_if.to_string());
    }
    i += 1;

    // Collect patterns (non-indented, non-assignment lines) and
    // assignments (indented lines starting with a field name)
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            // Blank line ends the if block
            i += 1;
            break;
        }

        let is_indented = line.starts_with(' ') || line.starts_with('\t');

        if is_indented {
            // This is a field assignment within the if block
            if let Some((name, value)) = parse_field_assignment(trimmed) {
                assignments.insert(name, value);
            }
            i += 1;
        } else if trimmed.starts_with("if") || trimmed.starts_with('#') || trimmed.starts_with(';') {
            // Start of a new block or comment - stop here
            break;
        } else if parse_field_assignment(trimmed).is_some() {
            // Non-indented field assignment = start of new top-level rule, stop
            break;
        } else {
            // Pattern line (non-indented, not a known directive)
            if patterns.is_empty() || !assignments.is_empty() {
                // If we already have assignments, this is a new block
                if !assignments.is_empty() {
                    break;
                }
            }
            patterns.push(trimmed.to_string());
            i += 1;
        }
    }

    Ok((IfBlock { patterns, assignments }, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_rules() {
        let input = r#"
# Bank checking account
skip 1
fields date, description, amount, balance
date-format %m/%d/%Y
currency $
account1 assets:checking
"#;
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.skip, 1);
        assert_eq!(rules.fields_list, vec!["date", "description", "amount", "balance"]);
        assert_eq!(rules.date_format.as_deref(), Some("%m/%d/%Y"));
        assert_eq!(rules.currency.as_deref(), Some("$"));
        assert_eq!(rules.field_assignments.get("account1").unwrap(), "assets:checking");
    }

    #[test]
    fn parse_if_blocks() {
        let input = r#"
skip 1
fields date, description, amount
account1 assets:checking

if WHOLE FOODS
  account2 expenses:groceries

if SALARY
  account2 income:salary
"#;
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.if_blocks.len(), 2);
        assert_eq!(rules.if_blocks[0].patterns, vec!["WHOLE FOODS"]);
        assert_eq!(rules.if_blocks[0].assignments.get("account2").unwrap(), "expenses:groceries");
        assert_eq!(rules.if_blocks[1].patterns, vec!["SALARY"]);
        assert_eq!(rules.if_blocks[1].assignments.get("account2").unwrap(), "income:salary");
    }

    #[test]
    fn parse_multi_pattern_if() {
        let input = r#"
skip 1
fields date, description, amount

if
UBER
LYFT
  account2 expenses:transport
"#;
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.if_blocks.len(), 1);
        assert_eq!(rules.if_blocks[0].patterns, vec!["UBER", "LYFT"]);
        assert_eq!(rules.if_blocks[0].assignments.get("account2").unwrap(), "expenses:transport");
    }

    #[test]
    fn parse_separator_tab() {
        let input = "separator \\t\nskip 1\nfields date, description, amount\n";
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.separator, '\t');
    }

    #[test]
    fn parse_newest_first() {
        let input = "newest-first\nskip 1\nfields date, description, amount\n";
        let rules = parse_csv_rules(input).unwrap();
        assert!(rules.newest_first);
    }

    #[test]
    fn parse_decimal_mark() {
        let input = "decimal-mark ,\nskip 1\nfields date, description, amount\n";
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.decimal_mark, Some(','));
    }
}
