use crate::amount::parse_amount;
use crate::ast::*;
use crate::date::parse_date;
use crate::error::ParseError;

/// Parse a journal file from a string.
pub fn parse(input: &str) -> Result<Journal, ParseError> {
    // If input is empty, return empty journal
    if input.trim().is_empty() {
        return Ok(Journal {
            items: vec![],
            source_path: None,
        });
    }

    // For now, use a line-based parser that handles the core syntax
    // without relying on pest for the full grammar (we'll migrate to pest
    // once the grammar is fully validated).
    parse_lines(input)
}

/// Line-based parser for core hledger journal syntax.
fn parse_lines(input: &str) -> Result<Journal, ParseError> {
    let mut items = Vec::new();
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            items.push(JournalItem::BlankLine);
            i += 1;
        } else if trimmed.starts_with(';') || trimmed.starts_with('#') {
            items.push(JournalItem::Comment(Comment {
                text: trimmed.to_string(),
            }));
            i += 1;
        } else if trimmed.starts_with("account ") {
            let name = trimmed.strip_prefix("account ").unwrap().trim();
            let (name, comment) = split_inline_comment(name);
            items.push(JournalItem::AccountDirective(AccountDirective {
                name: AccountName::new(name.trim()),
                comment,
            }));
            i += 1;
        } else if trimmed.starts_with("commodity ") {
            let rest = trimmed.strip_prefix("commodity ").unwrap().trim();
            items.push(JournalItem::CommodityDirective(CommodityDirective {
                commodity: rest.to_string(),
                format: None,
            }));
            i += 1;
        } else if trimmed.starts_with("P ") {
            if let Some(pd) = parse_price_directive(trimmed) {
                items.push(JournalItem::PriceDirective(pd));
            }
            i += 1;
        } else if trimmed.starts_with("include ") {
            let path = trimmed.strip_prefix("include ").unwrap().trim();
            items.push(JournalItem::IncludeDirective(IncludeDirective {
                path: path.to_string(),
            }));
            i += 1;
        } else if trimmed.starts_with("alias ") {
            let rest = trimmed.strip_prefix("alias ").unwrap().trim();
            if let Some(eq_pos) = rest.find('=') {
                let from = rest[..eq_pos].trim().to_string();
                let to = rest[eq_pos + 1..].trim().to_string();
                items.push(JournalItem::AliasDirective(AliasDirective { from, to }));
            }
            i += 1;
        } else if trimmed.starts_with("decimal-mark ") {
            let mark = trimmed.strip_prefix("decimal-mark ").unwrap().trim();
            if let Some(ch) = mark.chars().next() {
                items.push(JournalItem::DecimalMarkDirective(DecimalMarkDirective {
                    mark: ch,
                }));
            }
            i += 1;
        } else if trimmed.starts_with('~') {
            // Periodic transaction
            let start_byte = byte_offset(input, i, &lines);
            let header = trimmed[1..].trim();
            let txn_start = i;
            i += 1;

            // Collect posting lines
            while i < lines.len()
                && !lines[i].is_empty()
                && (lines[i].starts_with(' ') || lines[i].starts_with('\t'))
            {
                i += 1;
            }

            let end_byte = byte_offset(input, i, &lines);
            let (period, description) = split_first_word(header);

            let mut postings = Vec::new();
            for (j, posting_line) in lines[txn_start + 1..i].iter().enumerate() {
                let pl = posting_line.trim();
                if pl.is_empty() || pl.starts_with(';') || pl.starts_with('#') {
                    continue;
                }
                if let Ok(posting) = parse_posting(pl, txn_start + j + 2) {
                    postings.push(posting);
                }
            }

            items.push(JournalItem::PeriodicTransaction(PeriodicTransaction {
                period: period.to_string(),
                description: description.trim().to_string(),
                postings,
                span: SourceSpan { start: start_byte, end: end_byte, line: txn_start + 1 },
            }));
        } else if trimmed.starts_with('=') && !trimmed.starts_with("==") {
            // Auto posting rule
            let start_byte = byte_offset(input, i, &lines);
            let query = trimmed[1..].trim().to_string();
            let txn_start = i;
            i += 1;

            while i < lines.len()
                && !lines[i].is_empty()
                && (lines[i].starts_with(' ') || lines[i].starts_with('\t'))
            {
                i += 1;
            }

            let end_byte = byte_offset(input, i, &lines);
            let mut postings = Vec::new();
            for (j, posting_line) in lines[txn_start + 1..i].iter().enumerate() {
                let pl = posting_line.trim();
                if pl.is_empty() || pl.starts_with(';') || pl.starts_with('#') {
                    continue;
                }
                if let Ok(posting) = parse_posting(pl, txn_start + j + 2) {
                    postings.push(posting);
                }
            }

            items.push(JournalItem::AutoPostingRule(AutoPostingRule {
                query,
                postings,
                span: SourceSpan { start: start_byte, end: end_byte, line: txn_start + 1 },
            }));
        } else if starts_with_date(trimmed) {
            // Try to parse as a transaction
            let start_byte = byte_offset(input, i, &lines);
            let txn_start = i;
            i += 1;

            // Collect posting lines (indented lines)
            while i < lines.len()
                && !lines[i].is_empty()
                && (lines[i].starts_with(' ') || lines[i].starts_with('\t'))
            {
                i += 1;
            }

            let end_byte = byte_offset(input, i, &lines);
            let txn_text: Vec<&str> = lines[txn_start..i].to_vec();

            match parse_transaction(&txn_text, txn_start + 1, start_byte, end_byte) {
                Ok(txn) => items.push(JournalItem::Transaction(txn)),
                Err(e) => return Err(e),
            }
        } else {
            // Unknown line - treat as comment
            items.push(JournalItem::Comment(Comment {
                text: line.to_string(),
            }));
            i += 1;
        }
    }

    Ok(Journal {
        items,
        source_path: None,
    })
}

/// Parse a single transaction from its constituent lines.
fn parse_transaction(
    lines: &[&str],
    line_number: usize,
    start_byte: usize,
    end_byte: usize,
) -> Result<Transaction, ParseError> {
    if lines.is_empty() {
        return Err(ParseError::Syntax {
            line: line_number,
            message: "empty transaction".to_string(),
        });
    }

    let header = lines[0].trim();

    // Parse header: DATE [=DATE2] [STATUS] [CODE] DESCRIPTION [; COMMENT]
    let (header, comment) = split_inline_comment(header);
    let tags = comment
        .as_ref()
        .map(|c| parse_tags(&c.text))
        .unwrap_or_default();

    let mut parts = header.trim();

    // Parse date (may include =DATE2 for secondary date)
    let (first_word, rest) = split_first_word(parts);

    // Check for secondary date: 2024-01-15=2024-01-16
    let (date_str, secondary_date) = if let Some(eq_pos) = first_word.find('=') {
        let d1 = &first_word[..eq_pos];
        let d2 = &first_word[eq_pos + 1..];
        (d1, Some(parse_date(d2)?))
    } else {
        (first_word, None)
    };

    let date = parse_date(date_str)?;
    parts = rest.trim();

    // Parse optional status
    let mut status = Status::Unmarked;
    if parts.starts_with('!') {
        status = Status::Pending;
        parts = parts[1..].trim();
    } else if parts.starts_with('*') {
        status = Status::Cleared;
        parts = parts[1..].trim();
    }

    // Parse optional code
    let mut code = None;
    if parts.starts_with('(') {
        if let Some(close) = parts.find(')') {
            code = Some(parts[1..close].to_string());
            parts = parts[close + 1..].trim();
        }
    }

    // Rest is description
    let description = parts.trim().to_string();

    // Parse postings
    let mut postings = Vec::new();
    for (j, posting_line) in lines[1..].iter().enumerate() {
        let posting_line = posting_line.trim();
        if posting_line.is_empty() {
            continue;
        }
        if posting_line.starts_with(';') || posting_line.starts_with('#') {
            // Transaction comment line - skip for now
            continue;
        }

        let posting_line_num = line_number + j + 1;
        let posting = parse_posting(posting_line, posting_line_num)?;
        postings.push(posting);
    }

    Ok(Transaction {
        span: SourceSpan {
            start: start_byte,
            end: end_byte,
            line: line_number,
        },
        date,
        secondary_date,
        status,
        code,
        description,
        comment,
        tags,
        postings,
    })
}

/// Parse a single posting line.
fn parse_posting(line: &str, line_number: usize) -> Result<Posting, ParseError> {
    let line = line.trim();
    let (line, comment) = split_inline_comment(line);
    let tags = comment
        .as_ref()
        .map(|c| parse_tags(&c.text))
        .unwrap_or_default();
    let line = line.trim();

    // Parse status
    let (status, rest) = if line.starts_with('!') {
        (Status::Pending, line[1..].trim())
    } else if line.starts_with('*') {
        (Status::Cleared, line[1..].trim())
    } else {
        (Status::Unmarked, line)
    };

    // Check for virtual postings
    let (is_virtual, virtual_balanced, rest) = if rest.starts_with('(') {
        // Find matching close paren
        if let Some(close) = rest.find(')') {
            (true, false, rest[1..close].to_string() + &rest[close + 1..])
        } else {
            (false, false, rest.to_string())
        }
    } else if rest.starts_with('[') {
        if let Some(close) = rest.find(']') {
            (true, true, rest[1..close].to_string() + &rest[close + 1..])
        } else {
            (false, false, rest.to_string())
        }
    } else {
        (false, false, rest.to_string())
    };

    let rest = rest.trim();

    // Split account from amount using the two-space rule
    // The account name ends where we find 2+ consecutive spaces or a tab
    let (account_str, amount_str) = split_account_amount(rest);

    let account = AccountName::new(account_str.trim());

    // Parse balance assertion from amount string
    let (amount_str, balance_assertion) = extract_balance_assertion(amount_str.trim());

    // Parse amount (if present)
    let amount = if amount_str.trim().is_empty() {
        None
    } else {
        // Check for cost notation
        let (amt_part, cost) = extract_cost(amount_str.trim());
        let mut parsed = parse_amount(amt_part.trim())
            .map_err(|_| ParseError::Syntax {
                line: line_number,
                message: format!("invalid amount: {}", amount_str),
            })?;
        parsed.cost = cost;
        Some(parsed)
    };

    Ok(Posting {
        span: SourceSpan {
            start: 0,
            end: 0,
            line: line_number,
        },
        status,
        account,
        amount,
        balance_assertion,
        comment,
        tags,
        is_virtual,
        virtual_balanced,
    })
}

/// Split account name from amount using the two-space rule.
fn split_account_amount(s: &str) -> (&str, &str) {
    // Find the first occurrence of two consecutive spaces or a tab
    // after the start of the account name
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'\t' {
            return (&s[..i], &s[i + 1..]);
        }
        if i + 1 < bytes.len() && bytes[i] == b' ' && bytes[i + 1] == b' ' {
            return (&s[..i], &s[i..].trim_start());
        }
    }
    // No amount found - the entire string is the account name
    (s, "")
}

/// Extract cost notation (@ or @@) from an amount string.
fn extract_cost(s: &str) -> (&str, Option<Cost>) {
    // Look for @@ first (total cost), then @ (unit cost)
    if let Some(pos) = s.find("@@") {
        let amt = s[..pos].trim();
        let cost_str = s[pos + 2..].trim();
        if let Ok(cost_amt) = parse_amount(cost_str) {
            return (
                amt,
                Some(Cost::TotalCost(CostAmount {
                    quantity: cost_amt.quantity,
                    commodity: cost_amt.commodity,
                })),
            );
        }
    } else if let Some(pos) = s.find('@') {
        let amt = s[..pos].trim();
        let cost_str = s[pos + 1..].trim();
        if let Ok(cost_amt) = parse_amount(cost_str) {
            return (
                amt,
                Some(Cost::UnitCost(CostAmount {
                    quantity: cost_amt.quantity,
                    commodity: cost_amt.commodity,
                })),
            );
        }
    }
    (s, None)
}

/// Extract balance assertion (= or ==) from the end of an amount string.
fn extract_balance_assertion(s: &str) -> (&str, Option<BalanceAssertion>) {
    // Look for == first (strong), then = (normal)
    if let Some(pos) = s.find("==") {
        let before = s[..pos].trim();
        let assertion_str = s[pos + 2..].trim();
        if let Ok(amt) = parse_amount(assertion_str) {
            return (
                before,
                Some(BalanceAssertion {
                    strong: true,
                    quantity: amt.quantity,
                    commodity: amt.commodity,
                }),
            );
        }
    } else if let Some(pos) = s.find('=') {
        // Make sure this isn't part of == or a secondary date
        let before = s[..pos].trim();
        let assertion_str = s[pos + 1..].trim();
        if !assertion_str.is_empty() {
            if let Ok(amt) = parse_amount(assertion_str) {
                return (
                    before,
                    Some(BalanceAssertion {
                        strong: false,
                        quantity: amt.quantity,
                        commodity: amt.commodity,
                    }),
                );
            }
        }
    }
    (s, None)
}

/// Split inline comment from text.
fn split_inline_comment(s: &str) -> (&str, Option<Comment>) {
    // Find ; that's not inside quoted commodity names
    if let Some(pos) = s.find(';') {
        let before = &s[..pos];
        let comment_text = s[pos + 1..].trim();
        (
            before,
            Some(Comment {
                text: comment_text.to_string(),
            }),
        )
    } else {
        (s, None)
    }
}

/// Parse tags from a comment text (key:value pairs separated by commas).
fn parse_tags(comment: &str) -> Vec<Tag> {
    let mut tags = Vec::new();
    for part in comment.split(',') {
        let part = part.trim();
        if let Some(colon_pos) = part.find(':') {
            let name = part[..colon_pos].trim().to_string();
            let value = part[colon_pos + 1..].trim();
            if !name.is_empty() {
                tags.push(Tag {
                    name,
                    value: if value.is_empty() {
                        None
                    } else {
                        Some(value.to_string())
                    },
                });
            }
        }
    }
    tags
}

/// Check if a line starts with a date pattern.
fn starts_with_date(s: &str) -> bool {
    let s = s.trim();
    if s.len() < 8 {
        return false;
    }
    // Must start with a digit
    let first = s.as_bytes()[0];
    if !first.is_ascii_digit() {
        return false;
    }
    // Check if it looks like YYYY-MM-DD or similar
    s.chars()
        .take(10)
        .all(|c| c.is_ascii_digit() || c == '-' || c == '/' || c == '.')
        || s.chars().take(4).all(|c| c.is_ascii_digit())
}

/// Split at the first whitespace.
fn split_first_word(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(pos) => (&s[..pos], &s[pos + 1..]),
        None => (s, ""),
    }
}

/// Calculate byte offset for line index.
fn byte_offset(input: &str, line_idx: usize, lines: &[&str]) -> usize {
    let mut offset = 0;
    for (i, line) in lines.iter().enumerate() {
        if i >= line_idx {
            break;
        }
        offset += line.len() + 1; // +1 for newline
    }
    offset.min(input.len())
}

fn parse_price_directive(line: &str) -> Option<PriceDirective> {
    // P DATE COMMODITY PRICE
    let rest = line.strip_prefix("P ")?.trim();
    let (date_str, rest) = split_first_word(rest);
    let date = parse_date(date_str).ok()?;
    let rest = rest.trim();

    // The commodity is the next word
    let (commodity_str, price_str) = split_first_word(rest);
    let price_str = price_str.trim();

    if price_str.is_empty() {
        return None;
    }

    let price = parse_amount(price_str).ok()?;

    Some(PriceDirective {
        date,
        commodity: commodity_str.to_string(),
        price_quantity: price.quantity,
        price_commodity: price.commodity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    #[test]
    fn parse_empty_journal() {
        let journal = parse("").unwrap();
        assert!(journal.items.is_empty());
    }

    #[test]
    fn parse_comment_only() {
        let journal = parse("; this is a comment\n").unwrap();
        assert_eq!(journal.items.len(), 1);
        match &journal.items[0] {
            JournalItem::Comment(c) => assert_eq!(c.text, "; this is a comment"),
            _ => panic!("expected comment"),
        }
    }

    #[test]
    fn parse_simple_transaction() {
        let input = "2024-01-15 Grocery Store\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = parse(input).unwrap();

        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            other => panic!("expected transaction, got {:?}", other),
        };

        assert_eq!(txn.date, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
        assert_eq!(txn.description, "Grocery Store");
        assert_eq!(txn.status, Status::Unmarked);
        assert_eq!(txn.postings.len(), 2);

        assert_eq!(txn.postings[0].account.full, "expenses:food");
        let amt = txn.postings[0].amount.as_ref().unwrap();
        assert_eq!(amt.quantity, dec!(50.00));
        assert_eq!(amt.commodity, "$");

        assert_eq!(txn.postings[1].account.full, "assets:checking");
        assert!(txn.postings[1].amount.is_none());
    }

    #[test]
    fn parse_transaction_with_status() {
        let input = "2024-01-15 * Cleared transaction\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };
        assert_eq!(txn.status, Status::Cleared);
        assert_eq!(txn.description, "Cleared transaction");
    }

    #[test]
    fn parse_pending_transaction() {
        let input = "2024-01-15 ! Pending transaction\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };
        assert_eq!(txn.status, Status::Pending);
    }

    #[test]
    fn parse_transaction_with_code() {
        let input = "2024-01-15 (1234) Payee\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };
        assert_eq!(txn.code, Some("1234".to_string()));
        assert_eq!(txn.description, "Payee");
    }

    #[test]
    fn parse_multicurrency_transaction() {
        let input = "2024-01-15 Exchange\n    assets:eur  100.00 EUR\n    assets:usd  -110.00 USD\n";
        let journal = parse(input).unwrap();
        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };
        assert_eq!(txn.postings.len(), 2);

        let p1 = &txn.postings[0];
        assert_eq!(p1.account.full, "assets:eur");
        assert_eq!(p1.amount.as_ref().unwrap().quantity, dec!(100.00));
        assert_eq!(p1.amount.as_ref().unwrap().commodity, "EUR");

        let p2 = &txn.postings[1];
        assert_eq!(p2.account.full, "assets:usd");
        assert_eq!(p2.amount.as_ref().unwrap().quantity, dec!(-110.00));
        assert_eq!(p2.amount.as_ref().unwrap().commodity, "USD");
    }

    #[test]
    fn parse_transaction_with_cost() {
        let input =
            "2024-01-15 Exchange\n    assets:eur  100.00 EUR @ $1.10\n    assets:usd\n";
        let journal = parse(input).unwrap();
        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };

        let amt = txn.postings[0].amount.as_ref().unwrap();
        assert_eq!(amt.quantity, dec!(100.00));
        assert_eq!(amt.commodity, "EUR");

        match &amt.cost {
            Some(Cost::UnitCost(c)) => {
                assert_eq!(c.quantity, dec!(1.10));
                assert_eq!(c.commodity, "$");
            }
            other => panic!("expected unit cost, got {:?}", other),
        }
    }

    #[test]
    fn parse_transaction_with_total_cost() {
        let input =
            "2024-01-15 Exchange\n    assets:eur  100.00 EUR @@ $110.00\n    assets:usd\n";
        let journal = parse(input).unwrap();
        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };

        let amt = txn.postings[0].amount.as_ref().unwrap();
        match &amt.cost {
            Some(Cost::TotalCost(c)) => {
                assert_eq!(c.quantity, dec!(110.00));
                assert_eq!(c.commodity, "$");
            }
            other => panic!("expected total cost, got {:?}", other),
        }
    }

    #[test]
    fn parse_transaction_with_inline_comment() {
        let input =
            "2024-01-15 Grocery ; category:food\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };

        assert_eq!(txn.description, "Grocery");
        assert!(txn.comment.is_some());
        assert_eq!(txn.tags.len(), 1);
        assert_eq!(txn.tags[0].name, "category");
        assert_eq!(txn.tags[0].value, Some("food".to_string()));
    }

    #[test]
    fn parse_multiple_transactions() {
        let input = "\
2024-01-15 Transaction 1
    expenses:food  $50.00
    assets:checking

2024-01-16 Transaction 2
    expenses:rent  $1000.00
    assets:checking
";
        let journal = parse(input).unwrap();

        let txn_count = journal
            .items
            .iter()
            .filter(|i| matches!(i, JournalItem::Transaction(_)))
            .count();
        assert_eq!(txn_count, 2);
    }

    #[test]
    fn parse_account_hierarchy() {
        let name = AccountName::new("assets:bank:checking");
        assert_eq!(name.parts, vec!["assets", "bank", "checking"]);
        assert_eq!(name.depth(), 3);
    }

    #[test]
    fn parse_price_directive() {
        let input = "P 2024-01-15 AAPL $150.00\n";
        let journal = parse(input).unwrap();
        match &journal.items[0] {
            JournalItem::PriceDirective(pd) => {
                assert_eq!(pd.date, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
                assert_eq!(pd.commodity, "AAPL");
                assert_eq!(pd.price_quantity, dec!(150.00));
                assert_eq!(pd.price_commodity, "$");
            }
            _ => panic!("expected price directive"),
        }
    }

    #[test]
    fn parse_account_directive() {
        let input = "account assets:bank:checking\n";
        let journal = parse(input).unwrap();
        match &journal.items[0] {
            JournalItem::AccountDirective(ad) => {
                assert_eq!(ad.name.full, "assets:bank:checking");
            }
            _ => panic!("expected account directive"),
        }
    }

    #[test]
    fn parse_date_formats() {
        // All three date separators should work
        for sep in &["-", "/", "."] {
            let input = format!(
                "2024{}01{}15 Test\n    expenses:food  $50.00\n    assets:checking\n",
                sep, sep
            );
            let journal = parse(&input).unwrap();
            let txn = match &journal.items[0] {
                JournalItem::Transaction(t) => t,
                _ => panic!("expected transaction"),
            };
            assert_eq!(txn.date, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
        }
    }

    #[test]
    fn parse_posting_with_balance_assertion() {
        let input = "2024-01-15 Opening\n    assets:checking  $1000 = $1000\n    equity:opening\n";
        let journal = parse(input).unwrap();
        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };

        let p = &txn.postings[0];
        assert_eq!(p.amount.as_ref().unwrap().quantity, dec!(1000));
        let assertion = p.balance_assertion.as_ref().unwrap();
        assert!(!assertion.strong);
        assert_eq!(assertion.quantity, dec!(1000));
        assert_eq!(assertion.commodity, "$");
    }

    #[test]
    fn parse_secondary_date() {
        let input = "2024-01-15=2024-01-16 Test\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let txn = match &journal.items[0] {
            JournalItem::Transaction(t) => t,
            _ => panic!("expected transaction"),
        };
        assert_eq!(txn.date, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
        assert_eq!(
            txn.secondary_date,
            Some(NaiveDate::from_ymd_opt(2024, 1, 16).unwrap())
        );
    }
}
