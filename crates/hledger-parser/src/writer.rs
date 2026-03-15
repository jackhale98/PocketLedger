use crate::ast::*;

/// Configuration for how to format journal output.
#[derive(Debug, Clone)]
pub struct WriterConfig {
    /// Number of spaces for posting indentation.
    pub indent: usize,
    /// Minimum width for account names (right-padded with spaces).
    pub account_width: usize,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            indent: 4,
            account_width: 36,
        }
    }
}

/// Infer a WriterConfig from existing journal text by examining indentation patterns.
pub fn infer_config(text: &str) -> WriterConfig {
    let mut indent = 4usize;

    for line in text.lines() {
        if line.starts_with(' ') && !line.trim().is_empty() {
            let spaces = line.len() - line.trim_start().len();
            if spaces > 0 && spaces < indent {
                indent = spaces;
            }
        }
    }

    WriterConfig {
        indent,
        account_width: 36,
    }
}

/// Write a single transaction to hledger journal format.
pub fn write_transaction(txn: &Transaction, config: &WriterConfig) -> String {
    let mut out = String::new();

    // Date
    out.push_str(&txn.date.format("%Y-%m-%d").to_string());

    // Secondary date
    if let Some(ref d2) = txn.secondary_date {
        out.push('=');
        out.push_str(&d2.format("%Y-%m-%d").to_string());
    }

    // Status
    match txn.status {
        Status::Pending => out.push_str(" !"),
        Status::Cleared => out.push_str(" *"),
        Status::Unmarked => {}
    }

    // Code
    if let Some(ref code) = txn.code {
        out.push_str(&format!(" ({})", code));
    }

    // Description
    if !txn.description.is_empty() {
        out.push(' ');
        out.push_str(&txn.description);
    }

    // Inline comment
    if let Some(ref comment) = txn.comment {
        out.push_str(" ; ");
        out.push_str(&comment.text);
    }

    out.push('\n');

    // Postings
    for posting in &txn.postings {
        write_posting(&mut out, posting, config);
    }

    out
}

/// Write a single posting line.
fn write_posting(out: &mut String, posting: &Posting, config: &WriterConfig) {
    // Indentation
    for _ in 0..config.indent {
        out.push(' ');
    }

    // Status on posting
    match posting.status {
        Status::Pending => out.push_str("! "),
        Status::Cleared => out.push_str("* "),
        Status::Unmarked => {}
    }

    // Account name (with virtual wrapping)
    let account_str = if posting.is_virtual {
        if posting.virtual_balanced {
            format!("[{}]", posting.account.full)
        } else {
            format!("({})", posting.account.full)
        }
    } else {
        posting.account.full.clone()
    };

    out.push_str(&account_str);

    // Amount
    if let Some(ref amt) = posting.amount {
        // Pad account name to account_width
        let padding = if account_str.len() < config.account_width {
            config.account_width - account_str.len()
        } else {
            2 // minimum 2 spaces
        };
        for _ in 0..padding {
            out.push(' ');
        }

        out.push_str(&format_amount(amt));

        // Cost notation
        if let Some(ref cost) = amt.cost {
            match cost {
                Cost::UnitCost(c) => {
                    out.push_str(" @ ");
                    out.push_str(&format_cost_amount(c));
                }
                Cost::TotalCost(c) => {
                    out.push_str(" @@ ");
                    out.push_str(&format_cost_amount(c));
                }
            }
        }
    }

    // Balance assertion
    if let Some(ref assertion) = posting.balance_assertion {
        if assertion.strong {
            out.push_str(" == ");
        } else {
            out.push_str(" = ");
        }
        out.push_str(&format_simple_amount(
            assertion.quantity,
            &assertion.commodity,
            &AmountStyle::default(),
        ));
    }

    // Inline comment
    if let Some(ref comment) = posting.comment {
        out.push_str("  ; ");
        out.push_str(&comment.text);
    }

    out.push('\n');
}

/// Format an amount with its commodity according to its style.
fn format_amount(amt: &PostingAmount) -> String {
    format_simple_amount(amt.quantity, &amt.commodity, &amt.style)
}

/// Format a cost amount.
fn format_cost_amount(cost: &CostAmount) -> String {
    // Determine style heuristically
    let style = if is_symbol(&cost.commodity) {
        AmountStyle {
            commodity_side: Side::Left,
            commodity_spaced: false,
            decimal_mark: '.',
            precision: 2,
        }
    } else {
        AmountStyle {
            commodity_side: Side::Right,
            commodity_spaced: true,
            decimal_mark: '.',
            precision: 2,
        }
    };
    format_simple_amount(cost.quantity, &cost.commodity, &style)
}

/// Format a quantity with commodity.
fn format_simple_amount(
    quantity: rust_decimal::Decimal,
    commodity: &str,
    style: &AmountStyle,
) -> String {
    let num_str = format_decimal(quantity, style.precision);

    if commodity.is_empty() {
        return num_str;
    }

    match style.commodity_side {
        Side::Left => {
            if style.commodity_spaced {
                format!("{} {}", commodity, num_str)
            } else {
                format!("{}{}", commodity, num_str)
            }
        }
        Side::Right => {
            if style.commodity_spaced {
                format!("{} {}", num_str, commodity)
            } else {
                format!("{}{}", num_str, commodity)
            }
        }
    }
}

/// Format a Decimal to a string with a fixed number of decimal places.
fn format_decimal(value: rust_decimal::Decimal, precision: u8) -> String {
    if precision == 0 {
        // No decimal places
        let rounded = value.round_dp(0);
        return rounded.to_string();
    }

    let rounded = value.round_dp(precision as u32);
    let s = rounded.to_string();

    // Ensure we have the right number of decimal places
    if let Some(dot_pos) = s.find('.') {
        let decimals = s.len() - dot_pos - 1;
        if decimals < precision as usize {
            let padding = precision as usize - decimals;
            format!("{}{}", s, "0".repeat(padding))
        } else {
            s
        }
    } else {
        // No decimal point, add one
        format!("{}.{}", s, "0".repeat(precision as usize))
    }
}

/// Write a periodic transaction (budget) to hledger format.
pub fn write_periodic_transaction(
    period: &str,
    postings: &[(String, rust_decimal::Decimal, String)],
    config: &WriterConfig,
) -> String {
    let mut out = String::new();

    out.push_str("~ ");
    out.push_str(period);
    out.push('\n');

    for (account, quantity, commodity) in postings {
        // Indentation
        for _ in 0..config.indent {
            out.push(' ');
        }

        out.push_str(account);

        // Pad account name to account_width
        let padding = if account.len() < config.account_width {
            config.account_width - account.len()
        } else {
            2
        };
        for _ in 0..padding {
            out.push(' ');
        }

        // Format amount
        let style = if is_symbol(commodity) {
            AmountStyle {
                commodity_side: Side::Left,
                commodity_spaced: false,
                decimal_mark: '.',
                precision: 2,
            }
        } else if commodity.is_empty() {
            AmountStyle::default()
        } else {
            AmountStyle {
                commodity_side: Side::Right,
                commodity_spaced: true,
                decimal_mark: '.',
                precision: 2,
            }
        };

        out.push_str(&format_simple_amount(*quantity, commodity, &style));
        out.push('\n');
    }

    out
}

/// Check if a commodity string is a single-char currency symbol.
/// Patch a journal by replacing specific spans with new content.
/// Changes must be sorted by span start position (will be applied in reverse).
pub fn patch_journal(original: &str, changes: &[(SourceSpan, String)]) -> String {
    if changes.is_empty() {
        return original.to_string();
    }

    let mut result = original.to_string();

    // Apply changes in reverse order to preserve byte offsets
    let mut sorted_changes: Vec<&(SourceSpan, String)> = changes.iter().collect();
    sorted_changes.sort_by(|a, b| b.0.start.cmp(&a.0.start));

    for (span, replacement) in sorted_changes {
        let start = span.start.min(result.len());
        let end = span.end.min(result.len());
        result.replace_range(start..end, replacement);
    }

    result
}

/// Delete a transaction from a journal by its span, including surrounding blank lines.
pub fn delete_from_journal(original: &str, span: &SourceSpan) -> String {
    let start = span.start.min(original.len());
    let end = span.end.min(original.len());

    let mut result = String::new();
    result.push_str(&original[..start]);

    // Skip trailing newlines
    let remaining = &original[end..];
    let trimmed = remaining.trim_start_matches('\n');
    result.push_str(trimmed);

    result
}

fn is_symbol(commodity: &str) -> bool {
    let c = commodity.chars().next().unwrap_or('x');
    matches!(
        c,
        '$' | '€' | '£' | '¥' | '₹' | '₽' | '₿' | '₩' | '₫' | '₴' | '₸' | '₺' | '₦' | '₭'
    ) && commodity.chars().count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    fn make_posting(account: &str, quantity: rust_decimal::Decimal, commodity: &str) -> Posting {
        Posting {
            span: SourceSpan { start: 0, end: 0, line: 0 },
            status: Status::Unmarked,
            account: AccountName::new(account),
            amount: Some(PostingAmount {
                quantity,
                commodity: commodity.to_string(),
                style: if is_symbol(commodity) {
                    AmountStyle {
                        commodity_side: Side::Left,
                        commodity_spaced: false,
                        decimal_mark: '.',
                        precision: 2,
                    }
                } else if commodity.is_empty() {
                    AmountStyle::default()
                } else {
                    AmountStyle {
                        commodity_side: Side::Right,
                        commodity_spaced: true,
                        decimal_mark: '.',
                        precision: 2,
                    }
                },
                cost: None,
            }),
            balance_assertion: None,
            comment: None,
            tags: vec![],
            is_virtual: false,
            virtual_balanced: false,
        }
    }

    fn make_inferred_posting(account: &str) -> Posting {
        Posting {
            span: SourceSpan { start: 0, end: 0, line: 0 },
            status: Status::Unmarked,
            account: AccountName::new(account),
            amount: None,
            balance_assertion: None,
            comment: None,
            tags: vec![],
            is_virtual: false,
            virtual_balanced: false,
        }
    }

    fn make_txn(description: &str, postings: Vec<Posting>) -> Transaction {
        Transaction {
            span: SourceSpan { start: 0, end: 0, line: 0 },
            date: NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            secondary_date: None,
            status: Status::Unmarked,
            code: None,
            description: description.to_string(),
            comment: None,
            tags: vec![],
            postings,
        }
    }

    #[test]
    fn write_simple_transaction() {
        let txn = make_txn(
            "Grocery Store",
            vec![
                make_posting("expenses:food", dec!(50.00), "$"),
                make_inferred_posting("assets:checking"),
            ],
        );

        let output = write_transaction(&txn, &WriterConfig::default());

        // Verify structure
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines[0], "2024-01-15 Grocery Store");
        assert!(lines[1].starts_with("    expenses:food"));
        assert!(lines[1].contains("$50.00"));
        assert!(lines[1].ends_with("$50.00"));
        assert_eq!(lines[2].trim(), "assets:checking");

        // Verify round-trip parsability
        let reparsed = crate::parse(&output).unwrap();
        assert_eq!(reparsed.items.len(), 1);
    }

    #[test]
    fn write_cleared_transaction() {
        let mut txn = make_txn(
            "Cleared Purchase",
            vec![
                make_posting("expenses:food", dec!(25.00), "$"),
                make_inferred_posting("assets:checking"),
            ],
        );
        txn.status = Status::Cleared;

        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.starts_with("2024-01-15 * Cleared Purchase\n"));
    }

    #[test]
    fn write_pending_transaction() {
        let mut txn = make_txn(
            "Pending",
            vec![
                make_posting("expenses:food", dec!(10.00), "$"),
                make_inferred_posting("assets:cash"),
            ],
        );
        txn.status = Status::Pending;

        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.starts_with("2024-01-15 ! Pending\n"));
    }

    #[test]
    fn write_transaction_with_code() {
        let mut txn = make_txn(
            "Check Payment",
            vec![
                make_posting("expenses:rent", dec!(1200.00), "$"),
                make_inferred_posting("assets:checking"),
            ],
        );
        txn.code = Some("1001".to_string());

        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.starts_with("2024-01-15 (1001) Check Payment\n"));
    }

    #[test]
    fn write_multicurrency() {
        let txn = make_txn(
            "Exchange",
            vec![
                make_posting("assets:eur", dec!(100.00), "EUR"),
                make_posting("assets:usd", dec!(-110.00), "USD"),
            ],
        );

        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("100.00 EUR"));
        assert!(output.contains("-110.00 USD"));
    }

    #[test]
    fn write_with_unit_cost() {
        let mut posting = make_posting("assets:eur", dec!(100.00), "EUR");
        posting.amount.as_mut().unwrap().cost = Some(Cost::UnitCost(CostAmount {
            quantity: dec!(1.10),
            commodity: "$".to_string(),
        }));

        let txn = make_txn(
            "Exchange",
            vec![posting, make_inferred_posting("assets:usd")],
        );

        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("100.00 EUR @ $1.10"));
    }

    #[test]
    fn write_with_total_cost() {
        let mut posting = make_posting("assets:eur", dec!(100.00), "EUR");
        posting.amount.as_mut().unwrap().cost = Some(Cost::TotalCost(CostAmount {
            quantity: dec!(110.00),
            commodity: "$".to_string(),
        }));

        let txn = make_txn(
            "Exchange",
            vec![posting, make_inferred_posting("assets:usd")],
        );

        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("100.00 EUR @@ $110.00"));
    }

    #[test]
    fn write_with_inline_comment() {
        let mut txn = make_txn(
            "Grocery",
            vec![
                make_posting("expenses:food", dec!(50.00), "$"),
                make_inferred_posting("assets:checking"),
            ],
        );
        txn.comment = Some(Comment {
            text: "category:food".to_string(),
        });

        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("Grocery ; category:food"));
    }

    #[test]
    fn write_with_secondary_date() {
        let mut txn = make_txn(
            "Backdated",
            vec![
                make_posting("expenses:food", dec!(10.00), "$"),
                make_inferred_posting("assets:cash"),
            ],
        );
        txn.secondary_date = Some(NaiveDate::from_ymd_opt(2024, 1, 16).unwrap());

        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.starts_with("2024-01-15=2024-01-16 Backdated\n"));
    }

    #[test]
    fn write_with_balance_assertion() {
        let mut posting = make_posting("assets:checking", dec!(1000.00), "$");
        posting.balance_assertion = Some(BalanceAssertion {
            strong: false,
            quantity: dec!(1000.00),
            commodity: "$".to_string(),
        });

        let txn = make_txn(
            "Opening",
            vec![posting, make_inferred_posting("equity:opening")],
        );

        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("$1000.00 = $1000.00"));
    }

    #[test]
    fn roundtrip_parse_write_parse() {
        let input = "2024-01-15 Grocery Store\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = crate::parse(input).unwrap();

        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };

        let config = WriterConfig {
            indent: 4,
            account_width: 36,
        };
        let output = write_transaction(txn, &config);

        // Parse the output
        let reparsed = crate::parse(&output).unwrap();
        let reparsed_txn = match &reparsed.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };

        assert_eq!(txn.date, reparsed_txn.date);
        assert_eq!(txn.description, reparsed_txn.description);
        assert_eq!(txn.postings.len(), reparsed_txn.postings.len());
        assert_eq!(
            txn.postings[0].amount.as_ref().unwrap().quantity,
            reparsed_txn.postings[0].amount.as_ref().unwrap().quantity,
        );
    }

    #[test]
    fn format_decimal_precision() {
        assert_eq!(format_decimal(dec!(100), 2), "100.00");
        assert_eq!(format_decimal(dec!(100.5), 2), "100.50");
        assert_eq!(format_decimal(dec!(100.123), 2), "100.12");
        assert_eq!(format_decimal(dec!(100), 0), "100");
    }

    #[test]
    fn infer_config_from_text() {
        let text = "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking\n";
        let config = infer_config(text);
        assert_eq!(config.indent, 4);
    }
}
