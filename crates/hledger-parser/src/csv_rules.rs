use std::collections::HashMap;

use crate::error::ParseError;

/// A parsed CSV rules file.
#[derive(Debug, Clone, PartialEq)]
pub struct CsvRules {
    /// Number of header lines to skip (default 0, matching hledger).
    pub skip: usize,
    /// Field separator character (default ',').
    pub separator: char,
    /// Date format string (strftime-style, e.g. "%m/%d/%Y"). None = simple dates.
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
    /// Conditional blocks, evaluated in order (later matching blocks override earlier).
    pub if_blocks: Vec<IfBlock>,
    /// Non-fatal problems found while parsing (e.g. unknown directives).
    pub warnings: Vec<String>,
}

/// A conditional block: if the matchers match, apply the assignments.
#[derive(Debug, Clone, PartialEq)]
pub struct IfBlock {
    /// Matcher lines, kept verbatim (may carry `&`, `!`, `%field` prefixes).
    /// Consecutive lines form OR alternatives; a `&`-prefixed line ANDs with
    /// the preceding line's group. Interpretation happens at conversion time.
    pub patterns: Vec<String>,
    /// Field assignments to apply when matched.
    pub assignments: HashMap<String, String>,
}

impl Default for CsvRules {
    fn default() -> Self {
        Self {
            skip: 0,
            separator: ',',
            date_format: None,
            currency: None,
            decimal_mark: None,
            newest_first: false,
            fields_list: Vec::new(),
            field_assignments: HashMap::new(),
            if_blocks: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

/// Strip a directive name from a line, requiring the name to be a complete
/// token: it must be followed by whitespace or end-of-line. Returns the
/// trimmed remainder on match.
fn strip_directive<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(name)?;
    if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
        Some(rest.trim())
    } else {
        None
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

        // Directives (exact tokens: name must be followed by whitespace or EOL)
        if let Some(rest) = strip_directive(trimmed, "skip") {
            if rest.is_empty() {
                rules.skip = 1;
            } else {
                match rest.parse::<usize>() {
                    Ok(n) => rules.skip = n,
                    Err(_) => {
                        rules.warnings.push(format!(
                            "Line {}: invalid skip count '{}', using 1",
                            i + 1,
                            rest
                        ));
                        rules.skip = 1;
                    }
                }
            }
            i += 1;
        } else if let Some(rest) = strip_directive(trimmed, "separator") {
            rules.separator = match rest {
                "\\t" | "TAB" | "tab" => '\t',
                s if s.chars().count() == 1 => s.chars().next().unwrap(),
                _ => ',',
            };
            i += 1;
        } else if let Some(rest) = strip_directive(trimmed, "date-format") {
            rules.date_format = Some(rest.to_string());
            i += 1;
        } else if let Some(rest) = strip_directive(trimmed, "decimal-mark") {
            if let Some(c) = rest.chars().next() {
                rules.decimal_mark = Some(c);
            }
            i += 1;
        } else if strip_directive(trimmed, "newest-first").is_some() {
            rules.newest_first = true;
            i += 1;
        } else if let Some(rest) = strip_directive(trimmed, "fields") {
            let fields: Vec<String> = rest
                .split(',')
                .map(|f| f.trim().to_lowercase().to_string())
                .filter(|f| !f.is_empty())
                .collect();
            rules.fields_list = fields;
            i += 1;
        } else if let Some(rest) = strip_directive(trimmed, "currency") {
            rules.currency = Some(rest.to_string());
            i += 1;
        } else if strip_directive(trimmed, "if").is_some() {
            // Parse if block
            let (if_block, next_i) = parse_if_block(&lines, i)?;
            rules.if_blocks.push(if_block);
            i = next_i;
        } else if let Some((name, value)) = parse_field_assignment(trimmed) {
            rules.field_assignments.insert(name, value);
            i += 1;
        } else {
            // Unknown directive - record a warning instead of ignoring silently
            let token = trimmed.split_whitespace().next().unwrap_or(trimmed);
            rules
                .warnings
                .push(format!("Line {}: unknown directive '{}'", i + 1, token));
            i += 1;
        }
    }

    Ok(rules)
}

/// Non-numbered field names valid in assignments.
const FIELD_NAMES: &[&str] = &[
    "amount", "amount-in", "amount-out",
    "date", "date2", "description", "comment", "status", "code",
    "balance",
];

/// Is `name` a valid assignable field name?
/// Accepts the fixed names above plus account1-9, amount1-9, balance1-9,
/// comment1-9.
fn is_field_name(name: &str) -> bool {
    if FIELD_NAMES.contains(&name) {
        return true;
    }
    for prefix in ["account", "amount", "balance", "comment"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            if rest.len() == 1 && rest.chars().all(|c| c.is_ascii_digit()) && rest != "0" {
                return true;
            }
        }
    }
    false
}

fn parse_field_assignment(line: &str) -> Option<(String, String)> {
    let mut split = line.splitn(2, [' ', '\t']);
    let name = split.next()?;
    if !is_field_name(name) {
        return None;
    }
    let value = split.next().unwrap_or("").trim().to_string();
    // Require whitespace after the name (a bare field name is not an assignment)
    if line.len() == name.len() {
        return None;
    }
    Some((name.to_string(), value))
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
        } else if strip_directive(trimmed, "if").is_some()
            || trimmed.starts_with('#')
            || trimmed.starts_with(';')
        {
            // Start of a new block or comment - stop here
            break;
        } else if parse_field_assignment(trimmed).is_some() {
            // Non-indented field assignment = start of new top-level rule, stop
            break;
        } else {
            // Matcher line (non-indented, not a known directive).
            // If we already have assignments, this belongs to something else.
            if !assignments.is_empty() {
                break;
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
        assert!(rules.warnings.is_empty());
    }

    #[test]
    fn skip_defaults_to_zero() {
        // hledger's default is skip 0 (no header lines)
        let input = "fields date, description, amount\naccount1 assets:a\n";
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.skip, 0);
    }

    #[test]
    fn bare_skip_means_one() {
        let input = "skip\nfields date, description, amount\n";
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.skip, 1);
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
    fn parse_and_negation_matcher_lines_kept_verbatim() {
        let input = "skip 1\nfields date, description, amount\n\nif COFFEE\n& SHOP\n!TEA\n  account2 expenses:x\n";
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.if_blocks.len(), 1);
        assert_eq!(rules.if_blocks[0].patterns, vec!["COFFEE", "& SHOP", "!TEA"]);
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

    #[test]
    fn strict_directive_tokens() {
        // "skipfoo" must not be parsed as skip; it becomes a warning
        let input = "skipfoo 2\nfields date, description, amount\n";
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.skip, 0);
        assert_eq!(rules.warnings.len(), 1);
        assert!(rules.warnings[0].contains("skipfoo"));
    }

    #[test]
    fn unknown_directive_warns() {
        let input = "frobnicate yes\nskip 1\nfields date, description, amount\n";
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.skip, 1);
        assert_eq!(rules.warnings.len(), 1);
        assert!(rules.warnings[0].contains("frobnicate"));
    }

    #[test]
    fn numbered_amount_and_account_assignments() {
        let input = "skip 1\nfields date, description, amt\namount1 %amt\namount2 %amt\naccount3 assets:c\nbalance1 %amt\ncomment2 hi\n";
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.field_assignments.get("amount1").unwrap(), "%amt");
        assert_eq!(rules.field_assignments.get("amount2").unwrap(), "%amt");
        assert_eq!(rules.field_assignments.get("account3").unwrap(), "assets:c");
        assert_eq!(rules.field_assignments.get("balance1").unwrap(), "%amt");
        assert_eq!(rules.field_assignments.get("comment2").unwrap(), "hi");
        assert!(rules.warnings.is_empty());
    }

    #[test]
    fn amount1_in_if_block() {
        let input = "skip 1\nfields date, description, amt\n\nif FOO\n  amount1 %amt\n";
        let rules = parse_csv_rules(input).unwrap();
        assert_eq!(rules.if_blocks[0].assignments.get("amount1").unwrap(), "%amt");
    }
}
