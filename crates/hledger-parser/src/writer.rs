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

    // Inline comment; extra lines become indented comment lines below.
    let extra_txn_comment_lines = push_inline_comment(&mut out, txn.comment.as_ref(), " ; ");
    out.push('\n');
    for line in extra_txn_comment_lines {
        push_indent(&mut out, config.indent);
        out.push_str("; ");
        out.push_str(&line);
        out.push('\n');
    }

    // Postings
    for posting in &txn.postings {
        write_posting(&mut out, posting, config);
    }

    out
}

/// Push the first line of a comment inline; return any remaining lines.
fn push_inline_comment(out: &mut String, comment: Option<&Comment>, sep: &str) -> Vec<String> {
    match comment {
        Some(c) => {
            let mut lines = c.text.split('\n');
            if let Some(first) = lines.next() {
                out.push_str(sep);
                out.push_str(first);
            }
            lines.map(|l| l.to_string()).collect()
        }
        None => Vec::new(),
    }
}

fn push_indent(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push(' ');
    }
}

/// Write a single posting line.
fn write_posting(out: &mut String, posting: &Posting, config: &WriterConfig) {
    push_indent(out, config.indent);

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

    let has_amount = posting.amount.is_some();
    let account_chars = account_str.chars().count();

    // Amount
    if let Some(ref amt) = posting.amount {
        let padding = if account_chars < config.account_width {
            config.account_width - account_chars
        } else {
            2 // minimum 2 spaces
        };
        push_indent(out, padding);

        if amt.multiplier {
            out.push('*');
            out.push_str(&format_decimal(amt.quantity, amt.style.precision));
        } else {
            out.push_str(&format_amount(amt));
        }

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

    // Balance assertion (also written for assignments, where amount is None).
    if let Some(ref assertion) = posting.balance_assertion {
        if !has_amount {
            // Keep the two-space separation for `account  = AMOUNT` form.
            push_indent(out, 2);
        }
        out.push(' ');
        out.push('=');
        if assertion.strong {
            out.push('=');
        }
        if assertion.inclusive {
            out.push('*');
        }
        out.push(' ');
        out.push_str(&format_simple_amount(
            assertion.quantity,
            &assertion.commodity,
            &assertion.style,
        ));
    }

    // Inline comment; extra lines become their own comment lines.
    let extra = push_inline_comment(out, posting.comment.as_ref(), "  ; ");
    out.push('\n');
    for line in extra {
        push_indent(out, config.indent);
        out.push_str("; ");
        out.push_str(&line);
        out.push('\n');
    }
}

/// Format an amount with its commodity according to its style.
fn format_amount(amt: &PostingAmount) -> String {
    format_simple_amount(amt.quantity, &amt.commodity, &amt.style)
}

/// Format a cost amount using its captured style.
fn format_cost_amount(cost: &CostAmount) -> String {
    format_simple_amount(cost.quantity, &cost.commodity, &cost.style)
}

/// Build a sensible default style for a commodity (symbols left, codes right).
pub fn default_style_for(commodity: &str) -> AmountStyle {
    if is_symbol(commodity) {
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
    }
}

/// Format a quantity with commodity.
fn format_simple_amount(
    quantity: rust_decimal::Decimal,
    commodity: &str,
    style: &AmountStyle,
) -> String {
    let num_str = format_decimal(quantity, style.precision);

    // Commodity names containing spaces or digits must be quoted to re-parse.
    let needs_quotes = commodity
        .chars()
        .any(|c| c.is_whitespace() || c.is_ascii_digit() || c == ';');
    let commodity_str = if needs_quotes {
        format!("\"{}\"", commodity)
    } else {
        commodity.to_string()
    };

    if commodity.is_empty() {
        return num_str;
    }

    match style.commodity_side {
        Side::Left => {
            if style.commodity_spaced {
                format!("{} {}", commodity_str, num_str)
            } else {
                format!("{}{}", commodity_str, num_str)
            }
        }
        Side::Right => {
            if style.commodity_spaced {
                format!("{} {}", num_str, commodity_str)
            } else {
                format!("{}{}", num_str, commodity_str)
            }
        }
    }
}

/// Format a Decimal to a string with a fixed number of decimal places.
/// The value is never rounded away: if it has more decimals than `precision`,
/// the full value is written (destroying precision on disk is data loss).
fn format_decimal(value: rust_decimal::Decimal, precision: u8) -> String {
    let actual_scale = value.normalize().scale();
    let precision = (precision as u32).max(actual_scale);

    if precision == 0 {
        return value.normalize().to_string();
    }

    let s = value.normalize().to_string();
    if let Some(dot_pos) = s.find('.') {
        let decimals = s.len() - dot_pos - 1;
        if decimals < precision as usize {
            format!("{}{}", s, "0".repeat(precision as usize - decimals))
        } else {
            s
        }
    } else {
        format!("{}.{}", s, "0".repeat(precision as usize))
    }
}

/// Write a periodic transaction (budget) to hledger format.
/// `period` is the full period expression; postings carry their own styles.
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
        push_indent(&mut out, config.indent);
        out.push_str(account);

        let account_chars = account.chars().count();
        let padding = if account_chars < config.account_width {
            config.account_width - account_chars
        } else {
            2
        };
        push_indent(&mut out, padding);

        let style = default_style_for(commodity);
        out.push_str(&format_simple_amount(*quantity, commodity, &style));
        out.push('\n');
    }

    out
}

/// Patch a journal by replacing specific spans with new content.
/// Spans must lie within the text and must not overlap; a span outside the
/// text means the caller's view is stale and patching would corrupt the file.
pub fn patch_journal(
    original: &str,
    changes: &[(SourceSpan, String)],
) -> Result<String, String> {
    if changes.is_empty() {
        return Ok(original.to_string());
    }

    let mut sorted_changes: Vec<&(SourceSpan, String)> = changes.iter().collect();
    sorted_changes.sort_by(|a, b| b.0.start.cmp(&a.0.start));

    // Validate all spans before touching anything.
    let mut prev_start = usize::MAX;
    for (span, _) in &sorted_changes {
        if span.end > original.len() || span.start > span.end {
            return Err(format!(
                "stale edit: span {}..{} is outside the current file (len {})",
                span.start,
                span.end,
                original.len()
            ));
        }
        if span.end > prev_start {
            return Err(format!(
                "overlapping edit spans at {}..{}",
                span.start, span.end
            ));
        }
        if !original.is_char_boundary(span.start) || !original.is_char_boundary(span.end) {
            return Err(format!(
                "stale edit: span {}..{} does not fall on character boundaries",
                span.start, span.end
            ));
        }
        prev_start = span.start;
    }

    let mut result = original.to_string();
    for (span, replacement) in sorted_changes {
        result.replace_range(span.start..span.end, replacement);
    }

    Ok(result)
}

/// Delete a transaction from a journal by its span, including surrounding blank lines.
pub fn delete_from_journal(original: &str, span: &SourceSpan) -> Result<String, String> {
    if span.end > original.len() || span.start > span.end {
        return Err(format!(
            "stale delete: span {}..{} is outside the current file (len {})",
            span.start,
            span.end,
            original.len()
        ));
    }
    if !original.is_char_boundary(span.start) || !original.is_char_boundary(span.end) {
        return Err("stale delete: span does not fall on character boundaries".to_string());
    }

    let mut result = String::new();
    // Also remove the blank line(s) preceding the transaction so deletes in
    // the middle of a file don't accumulate double blanks.
    let before = &original[..span.start];
    let trimmed_before = before.trim_end_matches(|c| c == '\n' || c == '\r');
    if trimmed_before.is_empty() {
        // Deleting the first item: don't leave leading blank lines.
        result.push_str(trimmed_before);
    } else {
        result.push_str(trimmed_before);
        result.push('\n');
    }

    let remaining = &original[span.end..];
    let trimmed = remaining.trim_start_matches(|c| c == '\n' || c == '\r');
    if !trimmed.is_empty() && !result.is_empty() {
        result.push('\n');
    }
    result.push_str(trimmed);

    Ok(result)
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
                style: default_style_for(commodity),
                cost: None,
                multiplier: false,
            }),
            balance_assertion: None,
            comment: None,
            tags: vec![],
            is_virtual: false,
            virtual_balanced: false,
            date: None,
            date2: None,
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
            date: None,
            date2: None,
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

        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines[0], "2024-01-15 Grocery Store");
        assert!(lines[1].starts_with("    expenses:food"));
        assert!(lines[1].ends_with("$50.00"));
        assert_eq!(lines[2].trim(), "assets:checking");

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
            style: default_style_for("$"),
        }));

        let txn = make_txn("Exchange", vec![posting, make_inferred_posting("assets:usd")]);
        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("100.00 EUR @ $1.10"));
    }

    #[test]
    fn write_with_total_cost() {
        let mut posting = make_posting("assets:eur", dec!(100.00), "EUR");
        posting.amount.as_mut().unwrap().cost = Some(Cost::TotalCost(CostAmount {
            quantity: dec!(110.00),
            commodity: "$".to_string(),
            style: default_style_for("$"),
        }));

        let txn = make_txn("Exchange", vec![posting, make_inferred_posting("assets:usd")]);
        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("100.00 EUR @@ $110.00"));
    }

    #[test]
    fn cost_precision_preserved() {
        let mut posting = make_posting("assets:eur", dec!(100.00), "EUR");
        posting.amount.as_mut().unwrap().cost = Some(Cost::UnitCost(CostAmount {
            quantity: dec!(1.2345),
            commodity: "$".to_string(),
            style: AmountStyle {
                commodity_side: Side::Left,
                commodity_spaced: false,
                decimal_mark: '.',
                precision: 4,
            },
        }));

        let txn = make_txn("Exchange", vec![posting, make_inferred_posting("assets:usd")]);
        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("@ $1.2345"), "output: {}", output);
    }

    #[test]
    fn high_precision_never_rounded_away() {
        // Even with a style claiming precision 2, the real value must survive.
        let mut posting = make_posting("assets:btc", dec!(0.00012345), "BTC");
        posting.amount.as_mut().unwrap().style.precision = 2;

        let txn = make_txn("Buy", vec![posting, make_inferred_posting("assets:cash")]);
        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("0.00012345 BTC"), "output: {}", output);
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
    fn write_multiline_comments() {
        let mut txn = make_txn(
            "Grocery",
            vec![
                make_posting("expenses:food", dec!(50.00), "$"),
                make_inferred_posting("assets:checking"),
            ],
        );
        txn.comment = Some(Comment {
            text: "first line\nsecond line".to_string(),
        });
        txn.postings[0].comment = Some(Comment {
            text: "inline\nfollow-up".to_string(),
        });

        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("Grocery ; first line\n    ; second line\n"));
        assert!(output.contains("; inline\n    ; follow-up\n"));

        // Round-trips: comments and tags survive re-parsing.
        let reparsed = crate::parse(&output).unwrap();
        match &reparsed.items[0] {
            JournalItem::Transaction(t) => {
                assert_eq!(t.comment.as_ref().unwrap().text, "first line\nsecond line");
                assert_eq!(
                    t.postings[0].comment.as_ref().unwrap().text,
                    "inline\nfollow-up"
                );
            }
            _ => panic!("expected transaction"),
        }
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
            inclusive: false,
            quantity: dec!(1000.00),
            commodity: "$".to_string(),
            style: default_style_for("$"),
        });

        let txn = make_txn("Opening", vec![posting, make_inferred_posting("equity:opening")]);
        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("$1000.00 = $1000.00"));
    }

    #[test]
    fn write_inclusive_strong_assertion() {
        let mut posting = make_posting("assets", dec!(10.00), "$");
        posting.balance_assertion = Some(BalanceAssertion {
            strong: true,
            inclusive: true,
            quantity: dec!(160.00),
            commodity: "$".to_string(),
            style: default_style_for("$"),
        });

        let txn = make_txn("T", vec![posting, make_inferred_posting("equity")]);
        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("$10.00 ==* $160.00"), "output: {}", output);
    }

    #[test]
    fn write_quoted_commodity() {
        let txn = make_txn(
            "Fund buy",
            vec![
                make_posting("assets:funds", dec!(1.5), "VANGUARD FUND"),
                make_inferred_posting("assets:cash"),
            ],
        );
        let output = write_transaction(&txn, &WriterConfig::default());
        assert!(output.contains("1.50 \"VANGUARD FUND\""), "output: {}", output);
        // Must re-parse.
        let reparsed = crate::parse(&output).unwrap();
        match &reparsed.items[0] {
            JournalItem::Transaction(t) => {
                assert_eq!(t.postings[0].amount.as_ref().unwrap().commodity, "VANGUARD FUND")
            }
            _ => panic!("expected transaction"),
        }
    }

    #[test]
    fn roundtrip_parse_write_parse() {
        let input = "2024-01-15 Grocery Store\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = crate::parse(input).unwrap();

        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };

        let output = write_transaction(txn, &WriterConfig::default());
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
        // More real decimals than the style: keep them (no silent rounding).
        assert_eq!(format_decimal(dec!(100.123), 2), "100.123");
        assert_eq!(format_decimal(dec!(100), 0), "100");
    }

    #[test]
    fn patch_rejects_stale_spans() {
        let text = "short";
        let result = patch_journal(
            text,
            &[(SourceSpan { start: 0, end: 99, line: 1 }, "x".to_string())],
        );
        assert!(result.is_err());
    }

    #[test]
    fn patch_rejects_overlapping_spans() {
        let text = "0123456789";
        let result = patch_journal(
            text,
            &[
                (SourceSpan { start: 0, end: 5, line: 1 }, "a".to_string()),
                (SourceSpan { start: 3, end: 8, line: 1 }, "b".to_string()),
            ],
        );
        assert!(result.is_err());
    }

    #[test]
    fn patch_applies_in_order() {
        let text = "aaa bbb ccc";
        let result = patch_journal(
            text,
            &[
                (SourceSpan { start: 0, end: 3, line: 1 }, "XX".to_string()),
                (SourceSpan { start: 8, end: 11, line: 1 }, "YY".to_string()),
            ],
        )
        .unwrap();
        assert_eq!(result, "XX bbb YY");
    }

    #[test]
    fn delete_middle_transaction_no_double_blank() {
        let text = "2024-01-01 A\n    a  $1\n    b\n\n2024-01-02 B\n    a  $1\n    b\n\n2024-01-03 C\n    a  $1\n    b\n";
        let journal = crate::parse(text).unwrap();
        let spans: Vec<SourceSpan> = journal
            .items
            .iter()
            .filter_map(|i| match i {
                JournalItem::Transaction(t) => Some(t.span.clone()),
                _ => None,
            })
            .collect();
        let result = delete_from_journal(text, &spans[1]).unwrap();
        assert!(!result.contains("\n\n\n"), "result: {:?}", result);
        assert!(result.contains("2024-01-01 A"));
        assert!(!result.contains("2024-01-02 B"));
        assert!(result.contains("2024-01-03 C"));
    }

    #[test]
    fn infer_config_from_text() {
        let text = "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking\n";
        let config = infer_config(text);
        assert_eq!(config.indent, 4);
    }
}
