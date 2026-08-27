use chrono::Datelike;

use crate::amount::{parse_amount_ctx, parse_style_example, AmountContext};
use crate::ast::*;
use crate::date::parse_date_with_year;
use crate::error::ParseError;

/// Parse a journal file from a string.
pub fn parse(input: &str) -> Result<Journal, ParseError> {
    Parser::new(input).run()
}

struct CompiledAlias {
    to: String,
    matcher: AliasMatcher,
}

enum AliasMatcher {
    /// `alias OLD = NEW`: replaces OLD when it is the account or a prefix
    /// ending at a `:` boundary.
    Prefix(String),
    Regex(regex::Regex),
}

struct Parser<'a> {
    input: &'a str,
    /// Lines with any trailing '\r' stripped.
    lines: Vec<&'a str>,
    /// Byte offset of each line's start in the ORIGINAL input (CRLF-safe).
    line_starts: Vec<usize>,
    items: Vec<JournalItem>,
    warnings: Vec<ParseWarning>,

    // File-scoped parse state from directives.
    default_year: Option<i32>,
    amount_ctx: AmountContext,
    account_prefix: Vec<String>,
    aliases: Vec<CompiledAlias>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        let mut lines = Vec::new();
        let mut line_starts = Vec::new();
        let mut offset = 0usize;
        for raw in input.split('\n') {
            line_starts.push(offset);
            let stripped = raw.strip_suffix('\r').unwrap_or(raw);
            lines.push(stripped);
            offset += raw.len() + 1;
        }
        // split('\n') yields a final empty piece when input ends with '\n';
        // drop it so line indices match visible lines.
        if input.ends_with('\n') {
            lines.pop();
            line_starts.pop();
        }

        Self {
            input,
            lines,
            line_starts,
            items: Vec::new(),
            warnings: Vec::new(),
            default_year: None,
            amount_ctx: AmountContext::default(),
            account_prefix: Vec::new(),
            aliases: Vec::new(),
        }
    }

    /// Byte offset of the start of line `idx` (or end of input past the last line).
    fn line_start(&self, idx: usize) -> usize {
        self.line_starts
            .get(idx)
            .copied()
            .unwrap_or(self.input.len())
    }

    fn warn(&mut self, line: usize, message: impl Into<String>) {
        self.warnings.push(ParseWarning {
            line,
            message: message.into(),
        });
    }

    fn run(mut self) -> Result<Journal, ParseError> {
        if self.input.trim().is_empty() {
            return Ok(Journal {
                items: vec![],
                source_path: None,
                warnings: vec![],
            });
        }

        let mut i = 0;
        while i < self.lines.len() {
            i = self.parse_item(i)?;
        }

        Ok(Journal {
            items: self.items,
            source_path: None,
            warnings: self.warnings,
        })
    }

    /// Parse the item starting at line `i`; return the index of the next line.
    fn parse_item(&mut self, i: usize) -> Result<usize, ParseError> {
        let line = self.lines[i];

        if line.trim().is_empty() {
            self.items.push(JournalItem::BlankLine);
            return Ok(i + 1);
        }

        // Full-line comments: ';' '#' anywhere-leading, '*' org headings at margin.
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with(';') || trimmed_start.starts_with('#') {
            self.items.push(JournalItem::Comment(Comment {
                text: line.to_string(),
            }));
            return Ok(i + 1);
        }
        if line.starts_with('*') {
            self.items.push(JournalItem::Comment(Comment {
                text: line.to_string(),
            }));
            return Ok(i + 1);
        }

        // Indented stray line at top level: preserved, but the user should know.
        if line.starts_with(' ') || line.starts_with('\t') {
            self.warn(
                i + 1,
                format!("unexpected indented line ignored: {}", line.trim()),
            );
            self.items.push(JournalItem::Comment(Comment {
                text: line.to_string(),
            }));
            return Ok(i + 1);
        }

        // `comment` ... `end comment` block: everything inside is inert.
        if line == "comment" || line.starts_with("comment ") {
            let mut j = i;
            loop {
                self.items.push(JournalItem::Comment(Comment {
                    text: self.lines[j].to_string(),
                }));
                j += 1;
                if j >= self.lines.len() {
                    break;
                }
                if self.lines[j].trim_start().starts_with("end comment") {
                    self.items.push(JournalItem::Comment(Comment {
                        text: self.lines[j].to_string(),
                    }));
                    j += 1;
                    break;
                }
            }
            return Ok(j);
        }

        // Ledger-style '!'-prefixed directives: same meaning, strip the '!'.
        let (directive_line, _bang) = match line.strip_prefix('!') {
            Some(rest) => (rest, true),
            None => (line, false),
        };

        if let Some(next) = self.try_parse_directive(directive_line, i)? {
            return Ok(next);
        }

        if line.starts_with('~') {
            return self.parse_periodic(i);
        }

        if line.starts_with('=') && !line.starts_with("==") {
            return self.parse_auto_rule(i);
        }

        if line.starts_with(|c: char| c.is_ascii_digit()) {
            return self.parse_transaction_block(i);
        }

        // Unrecognized: preserve for round-trip, but never silently.
        self.warn(
            i + 1,
            format!("unrecognized line ignored: {}", line.trim()),
        );
        self.items.push(JournalItem::Comment(Comment {
            text: line.to_string(),
        }));
        Ok(i + 1)
    }

    /// Try to handle a directive line. Returns Some(next_line) if handled.
    fn try_parse_directive(&mut self, line: &str, i: usize) -> Result<Option<usize>, ParseError> {
        if let Some(rest) = line.strip_prefix("account ") {
            return self.parse_account_directive(rest, i).map(Some);
        }
        if let Some(rest) = line.strip_prefix("commodity ") {
            return self.parse_commodity_directive(rest, i).map(Some);
        }
        if let Some(_rest) = line.strip_prefix("P ") {
            match self.parse_price_directive(line) {
                Some(pd) => {
                    self.amount_ctx
                        .commodity_marks
                        .entry(pd.commodity.clone())
                        .or_insert('.');
                    self.items.push(JournalItem::PriceDirective(pd));
                }
                None => {
                    self.warn(i + 1, format!("malformed P price directive: {}", line));
                    self.items.push(JournalItem::Comment(Comment {
                        text: line.to_string(),
                    }));
                }
            }
            return Ok(Some(i + 1));
        }
        if let Some(rest) = line.strip_prefix("include ") {
            self.items.push(JournalItem::IncludeDirective(IncludeDirective {
                path: strip_inline_comment(rest).trim().to_string(),
            }));
            return Ok(Some(i + 1));
        }
        if let Some(rest) = line.strip_prefix("alias ") {
            self.parse_alias_directive(strip_inline_comment(rest), i);
            return Ok(Some(i + 1));
        }
        if line.trim() == "end aliases" {
            self.aliases.clear();
            self.items.push(JournalItem::OtherDirective(line.to_string()));
            return Ok(Some(i + 1));
        }
        if let Some(rest) = line.strip_prefix("decimal-mark ") {
            if let Some(ch) = rest.trim().chars().next() {
                self.amount_ctx.decimal_mark = Some(ch);
                self.items
                    .push(JournalItem::DecimalMarkDirective(DecimalMarkDirective {
                        mark: ch,
                    }));
            } else {
                self.warn(i + 1, "malformed decimal-mark directive");
                self.items.push(JournalItem::Comment(Comment {
                    text: line.to_string(),
                }));
            }
            return Ok(Some(i + 1));
        }
        if let Some(rest) = line.strip_prefix("D ") {
            match parse_style_example(strip_inline_comment(rest).trim()) {
                Some((commodity, style)) if !commodity.is_empty() => {
                    self.amount_ctx
                        .commodity_marks
                        .insert(commodity.clone(), style.decimal_mark);
                    self.amount_ctx.default_commodity = Some((commodity, style));
                }
                _ => self.warn(i + 1, format!("malformed D directive: {}", line)),
            }
            self.items.push(JournalItem::OtherDirective(line.to_string()));
            return Ok(Some(i + 1));
        }
        // Y / year directive: default year for partial dates.
        let year_rest = line
            .strip_prefix("year ")
            .or_else(|| line.strip_prefix("Y "))
            .or_else(|| if line.starts_with('Y') && line[1..].trim().chars().all(|c| c.is_ascii_digit()) && !line[1..].trim().is_empty() { Some(&line[1..]) } else { None });
        if let Some(rest) = year_rest {
            match strip_inline_comment(rest).trim().parse::<i32>() {
                Ok(y) => self.default_year = Some(y),
                Err(_) => self.warn(i + 1, format!("malformed year directive: {}", line)),
            }
            self.items.push(JournalItem::OtherDirective(line.to_string()));
            return Ok(Some(i + 1));
        }
        if let Some(rest) = line.strip_prefix("apply account ") {
            self.account_prefix
                .push(strip_inline_comment(rest).trim().to_string());
            self.items.push(JournalItem::OtherDirective(line.to_string()));
            return Ok(Some(i + 1));
        }
        if line.trim() == "end apply account" {
            if self.account_prefix.pop().is_none() {
                self.warn(i + 1, "end apply account without matching apply account");
            }
            self.items.push(JournalItem::OtherDirective(line.to_string()));
            return Ok(Some(i + 1));
        }
        // Recognized-but-inert directives: preserved verbatim, no warning.
        for prefix in ["payee ", "tag ", "note ", "define ", "bucket ", "A ", "C ", "N "] {
            if line.starts_with(prefix) {
                self.items.push(JournalItem::OtherDirective(line.to_string()));
                return Ok(Some(i + 1));
            }
        }
        Ok(None)
    }

    fn parse_account_directive(&mut self, rest: &str, i: usize) -> Result<usize, ParseError> {
        let (name_part, comment) = split_inline_comment(rest);
        let mut tags = comment
            .as_ref()
            .map(|c| parse_tags(&c.text))
            .unwrap_or_default();
        let name = AccountName::new(name_part.trim());

        // Consume indented subdirective lines (comments carry tags like type:).
        let mut j = i + 1;
        let mut extra_comments: Vec<String> = Vec::new();
        while j < self.lines.len()
            && !self.lines[j].trim().is_empty()
            && (self.lines[j].starts_with(' ') || self.lines[j].starts_with('\t'))
        {
            let sub = self.lines[j].trim();
            if let Some(text) = sub.strip_prefix(';') {
                tags.extend(parse_tags(text));
                extra_comments.push(text.trim().to_string());
            } else {
                // Unknown subdirective (note/alias/etc.) — preserve verbatim.
                self.items.push(JournalItem::OtherDirective(
                    self.lines[j].to_string(),
                ));
            }
            j += 1;
        }

        let merged_comment = match (comment, extra_comments.is_empty()) {
            (Some(c), true) => Some(c),
            (Some(c), false) => Some(Comment {
                text: format!("{}\n{}", c.text, extra_comments.join("\n")),
            }),
            (None, false) => Some(Comment {
                text: extra_comments.join("\n"),
            }),
            (None, true) => None,
        };

        self.items.push(JournalItem::AccountDirective(AccountDirective {
            name,
            comment: merged_comment,
            tags,
        }));
        Ok(j)
    }

    fn parse_commodity_directive(&mut self, rest: &str, i: usize) -> Result<usize, ParseError> {
        let (body, _comment) = split_inline_comment(rest);
        let body = body.trim();

        let mut commodity;
        let mut format = None;

        if body.contains(|c: char| c.is_ascii_digit()) {
            // Example-amount form: `commodity 1.000,00 EUR`
            match parse_style_example(body) {
                Some((c, style)) if !c.is_empty() => {
                    commodity = c;
                    format = Some(style);
                }
                _ => {
                    self.warn(i + 1, format!("malformed commodity directive: {}", body));
                    commodity = body.to_string();
                }
            }
        } else {
            // Name-only form, possibly quoted.
            commodity = body.trim_matches('"').to_string();
        }

        // Indented `format AMOUNT` subdirective.
        let mut j = i + 1;
        while j < self.lines.len()
            && !self.lines[j].trim().is_empty()
            && (self.lines[j].starts_with(' ') || self.lines[j].starts_with('\t'))
        {
            let sub = self.lines[j].trim();
            if let Some(fmt) = sub.strip_prefix("format ") {
                match parse_style_example(fmt.trim()) {
                    Some((c, style)) => {
                        if !c.is_empty() {
                            commodity = c;
                        }
                        format = Some(style);
                    }
                    None => self.warn(j + 1, format!("malformed commodity format: {}", fmt)),
                }
            } else if sub.starts_with(';') {
                // subdirective comment — ignore
            } else {
                self.items
                    .push(JournalItem::OtherDirective(self.lines[j].to_string()));
            }
            j += 1;
        }

        if let Some(ref style) = format {
            self.amount_ctx
                .commodity_marks
                .insert(commodity.clone(), style.decimal_mark);
        }

        self.items
            .push(JournalItem::CommodityDirective(CommodityDirective {
                commodity,
                format,
            }));
        Ok(j)
    }

    fn parse_alias_directive(&mut self, rest: &str, i: usize) {
        let rest = rest.trim();
        // Regex form: alias /RE/ = REPL
        if let Some(re_body) = rest.strip_prefix('/') {
            if let Some(close) = re_body.find('/') {
                let pattern = &re_body[..close];
                let after = re_body[close + 1..].trim();
                if let Some(repl) = after.strip_prefix('=') {
                    let repl = repl.trim();
                    match regex::Regex::new(pattern) {
                        Ok(re) => {
                            self.aliases.push(CompiledAlias {
                                to: repl.to_string(),
                                matcher: AliasMatcher::Regex(re),
                            });
                            self.items.push(JournalItem::AliasDirective(AliasDirective {
                                from: pattern.to_string(),
                                to: repl.to_string(),
                                regex: true,
                            }));
                        }
                        Err(e) => self.warn(i + 1, format!("invalid alias regex: {}", e)),
                    }
                    return;
                }
            }
            self.warn(i + 1, format!("malformed alias directive: alias {}", rest));
            return;
        }
        // Simple form: alias OLD = NEW
        if let Some(eq_pos) = rest.find('=') {
            let from = rest[..eq_pos].trim().to_string();
            let to = rest[eq_pos + 1..].trim().to_string();
            if from.is_empty() {
                self.warn(i + 1, format!("malformed alias directive: alias {}", rest));
                return;
            }
            self.aliases.push(CompiledAlias {
                to: to.clone(),
                matcher: AliasMatcher::Prefix(from.clone()),
            });
            self.items
                .push(JournalItem::AliasDirective(AliasDirective { from, to, regex: false }));
        } else {
            self.warn(i + 1, format!("malformed alias directive: alias {}", rest));
        }
    }

    /// Apply `apply account` prefix and aliases to an account name.
    fn resolve_account(&self, raw: &str) -> AccountName {
        let mut name = if self.account_prefix.is_empty() {
            raw.to_string()
        } else {
            format!("{}:{}", self.account_prefix.join(":"), raw)
        };

        for alias in &self.aliases {
            match &alias.matcher {
                AliasMatcher::Prefix(from) => {
                    if name == *from {
                        name = alias.to.clone();
                    } else if name.starts_with(from.as_str())
                        && name.as_bytes().get(from.len()) == Some(&b':')
                    {
                        name = format!("{}{}", alias.to, &name[from.len()..]);
                    }
                }
                AliasMatcher::Regex(re) => {
                    if re.is_match(&name) {
                        name = re.replace(&name, alias.to.as_str()).into_owned();
                    }
                }
            }
        }

        AccountName::new(&name)
    }

    /// Find the end (exclusive line index) of an indented block starting after `start`.
    fn block_end(&self, start: usize) -> usize {
        let mut j = start + 1;
        while j < self.lines.len()
            && !self.lines[j].is_empty()
            && (self.lines[j].starts_with(' ') || self.lines[j].starts_with('\t'))
        {
            j += 1;
        }
        j
    }

    fn parse_periodic(&mut self, i: usize) -> Result<usize, ParseError> {
        let end = self.block_end(i);
        let start_byte = self.line_start(i);
        let end_byte = self.line_start(end);

        let header = self.lines[i][1..].trim();
        // Period expression runs to the double-space (or tab) separator.
        let (period, description) = split_double_space(header);

        let postings = self.parse_posting_block(i + 1, end, false)?;

        self.items
            .push(JournalItem::PeriodicTransaction(PeriodicTransaction {
                period: period.trim().to_string(),
                description: description.trim().to_string(),
                postings,
                span: SourceSpan {
                    start: start_byte,
                    end: end_byte,
                    line: i + 1,
                },
            }));
        Ok(end)
    }

    fn parse_auto_rule(&mut self, i: usize) -> Result<usize, ParseError> {
        let end = self.block_end(i);
        let start_byte = self.line_start(i);
        let end_byte = self.line_start(end);

        let query = self.lines[i][1..].trim().to_string();
        let postings = self.parse_posting_block(i + 1, end, true)?;

        self.items.push(JournalItem::AutoPostingRule(AutoPostingRule {
            query,
            postings,
            span: SourceSpan {
                start: start_byte,
                end: end_byte,
                line: i + 1,
            },
        }));
        Ok(end)
    }

    /// Parse the posting lines of a periodic/auto block. Errors propagate —
    /// silently dropping postings falsifies budgets and auto rules.
    fn parse_posting_block(
        &mut self,
        start: usize,
        end: usize,
        allow_multiplier: bool,
    ) -> Result<Vec<Posting>, ParseError> {
        let mut postings = Vec::new();
        for j in start..end {
            let pl = self.lines[j].trim();
            if pl.is_empty() || pl.starts_with(';') || pl.starts_with('#') {
                continue;
            }
            let posting = self.parse_posting(pl, j + 1, allow_multiplier, None)?;
            postings.push(posting);
        }
        Ok(postings)
    }

    fn parse_transaction_block(&mut self, i: usize) -> Result<usize, ParseError> {
        let end = self.block_end(i);
        let start_byte = self.line_start(i);
        let end_byte = self.line_start(end);
        let line_number = i + 1;

        let header = self.lines[i].trim();
        let (header, mut comment) = split_inline_comment(header);
        let mut tags = comment
            .as_ref()
            .map(|c| parse_tags(&c.text))
            .unwrap_or_default();

        let mut parts = header.trim();

        // Date (may include =DATE2).
        let (first_word, rest) = split_first_word(parts);
        let (date_str, secondary_str) = match first_word.find('=') {
            Some(eq_pos) => (&first_word[..eq_pos], Some(&first_word[eq_pos + 1..])),
            None => (first_word, None),
        };

        let date = parse_date_with_year(date_str, self.default_year).map_err(|e| {
            ParseError::Syntax {
                line: line_number,
                message: e.to_string(),
            }
        })?;
        let secondary_date = match secondary_str {
            Some(s) => Some(
                parse_date_with_year(s, Some(date.year())).map_err(|e| ParseError::Syntax {
                    line: line_number,
                    message: e.to_string(),
                })?,
            ),
            None => None,
        };
        parts = rest.trim();

        // Status.
        let mut status = Status::Unmarked;
        if let Some(r) = parts.strip_prefix('!') {
            status = Status::Pending;
            parts = r.trim();
        } else if let Some(r) = parts.strip_prefix('*') {
            status = Status::Cleared;
            parts = r.trim();
        }

        // Code.
        let mut code = None;
        if parts.starts_with('(') {
            if let Some(close) = parts.find(')') {
                code = Some(parts[1..close].to_string());
                parts = parts[close + 1..].trim();
            }
        }

        let description = parts.trim().to_string();

        // Body: postings and comment lines. Comment lines attach to the last
        // posting (or the transaction, before the first posting) so they are
        // preserved and their tags captured.
        let mut postings: Vec<Posting> = Vec::new();
        for j in (i + 1)..end {
            let pl = self.lines[j].trim();
            if pl.is_empty() {
                continue;
            }
            if let Some(text) = pl.strip_prefix(';') {
                let text = text.trim();
                let line_tags = parse_tags(text);
                if let Some(last) = postings.last_mut() {
                    append_comment(&mut last.comment, text);
                    apply_posting_meta(last, &line_tags, date.year());
                    last.tags.extend(line_tags);
                } else {
                    append_comment(&mut comment, text);
                    tags.extend(line_tags);
                }
                continue;
            }
            if pl.starts_with('#') {
                // '#' is not a comment inside transactions in hledger; treat
                // like one anyway but tell the user.
                self.warn(j + 1, format!("'#' comment inside transaction: {}", pl));
                continue;
            }
            let posting = self.parse_posting(pl, j + 1, false, Some(date.year()))?;
            postings.push(posting);
        }

        self.items.push(JournalItem::Transaction(Transaction {
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
        }));
        Ok(end)
    }

    /// Parse a single posting line.
    fn parse_posting(
        &mut self,
        line: &str,
        line_number: usize,
        allow_multiplier: bool,
        txn_year: Option<i32>,
    ) -> Result<Posting, ParseError> {
        let line = line.trim();
        let (line, comment) = split_inline_comment(line);
        let tags = comment
            .as_ref()
            .map(|c| parse_tags(&c.text))
            .unwrap_or_default();
        let line = line.trim();

        // Status.
        let (status, rest) = if let Some(r) = line.strip_prefix('!') {
            (Status::Pending, r.trim())
        } else if let Some(r) = line.strip_prefix('*') {
            (Status::Cleared, r.trim())
        } else {
            (Status::Unmarked, line)
        };

        // Virtual postings.
        let (is_virtual, virtual_balanced, rest) = if rest.starts_with('(') {
            match rest.find(')') {
                Some(close) => {
                    if rest[1..close].trim().is_empty() {
                        return Err(ParseError::Syntax {
                            line: line_number,
                            message: "virtual posting with empty account name".to_string(),
                        });
                    }
                    (true, false, rest[1..close].to_string() + &rest[close + 1..])
                }
                None => (false, false, rest.to_string()),
            }
        } else if rest.starts_with('[') {
            match rest.find(']') {
                Some(close) => {
                    if rest[1..close].trim().is_empty() {
                        return Err(ParseError::Syntax {
                            line: line_number,
                            message: "virtual posting with empty account name".to_string(),
                        });
                    }
                    (true, true, rest[1..close].to_string() + &rest[close + 1..])
                }
                None => (false, false, rest.to_string()),
            }
        } else {
            (false, false, rest.to_string())
        };

        let rest = rest.trim();

        // Two-space rule separates account from amount.
        let (account_str, amount_str) = split_account_amount(rest);
        let account = self.resolve_account(account_str.trim());

        if account.full.is_empty() {
            return Err(ParseError::Syntax {
                line: line_number,
                message: "posting with empty account name".to_string(),
            });
        }

        // Balance assertion (searched outside braces/quotes so lot notation
        // and quoted commodities don't confuse it).
        let (amount_str, balance_assertion) =
            self.extract_balance_assertion(amount_str.trim(), line_number)?;

        // Amount (with optional cost / lot notation).
        let amount_str = amount_str.trim();
        let amount = if amount_str.is_empty() {
            None
        } else if allow_multiplier && amount_str.starts_with('*') {
            // Auto-posting multiplier: *N
            let q = crate::amount::parse_quantity_with(amount_str[1..].trim(), None).map_err(
                |_| ParseError::Syntax {
                    line: line_number,
                    message: format!("invalid multiplier amount: {}", amount_str),
                },
            )?;
            Some(PostingAmount {
                quantity: q.value,
                commodity: String::new(),
                style: AmountStyle {
                    commodity_side: Side::Left,
                    commodity_spaced: false,
                    decimal_mark: q.decimal_mark,
                    precision: q.precision,
                },
                cost: None,
                multiplier: true,
            })
        } else {
            let (amt_part, cost) = self.extract_cost(amount_str, line_number)?;
            let amt_part = amt_part.trim();
            if amt_part.is_empty() {
                // Balance assignment: `account  = AMOUNT` — no amount, only
                // the assertion; amount is computed from the running balance.
                None
            } else {
                let mut parsed =
                    parse_amount_ctx(amt_part, &self.amount_ctx).map_err(|_| {
                        ParseError::Syntax {
                            line: line_number,
                            message: format!("invalid amount: {}", amount_str),
                        }
                    })?;
                parsed.cost = cost;
                Some(parsed)
            }
        };

        let mut posting = Posting {
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
            tags: tags.clone(),
            is_virtual,
            virtual_balanced,
            date: None,
            date2: None,
        };
        apply_posting_meta(&mut posting, &tags, txn_year.unwrap_or(0));
        Ok(posting)
    }

    /// Extract a balance assertion (`=`, `==`, `=*`, `==*`) from the end of an
    /// amount string, ignoring '=' inside braces or quotes.
    fn extract_balance_assertion<'s>(
        &mut self,
        s: &'s str,
        line_number: usize,
    ) -> Result<(&'s str, Option<BalanceAssertion>), ParseError> {
        let Some(pos) = find_outside_delims(s, '=') else {
            return Ok((s, None));
        };

        let before = s[..pos].trim();
        let mut after = &s[pos + 1..];
        let strong = if let Some(r) = after.strip_prefix('=') {
            after = r;
            true
        } else {
            false
        };
        let inclusive = if let Some(r) = after.trim_start().strip_prefix('*') {
            after = r;
            true
        } else {
            false
        };
        let assertion_str = after.trim();

        if assertion_str.is_empty() {
            return Err(ParseError::Syntax {
                line: line_number,
                message: "balance assertion with no amount".to_string(),
            });
        }

        let amt = parse_amount_ctx(assertion_str, &self.amount_ctx).map_err(|_| {
            ParseError::Syntax {
                line: line_number,
                message: format!("invalid balance assertion amount: {}", assertion_str),
            }
        })?;

        Ok((
            before,
            Some(BalanceAssertion {
                strong,
                inclusive,
                quantity: amt.quantity,
                commodity: amt.commodity,
                style: amt.style,
            }),
        ))
    }

    /// Extract cost notation from an amount string. `@`/`@@` win over lot
    /// notation `{...}` (hledger 1.32 semantics); `{=...}` fixated lots are
    /// read like `{...}`.
    fn extract_cost<'s>(
        &mut self,
        s: &'s str,
        line_number: usize,
    ) -> Result<(String, Option<Cost>), ParseError> {
        // Pull out a lot-brace segment if present.
        let (without_lot, lot_cost) = if let Some(open) = s.find('{') {
            let total = s[open..].starts_with("{{");
            let close_pat = if total { "}}" } else { "}" };
            let open_len = if total { 2 } else { 1 };
            match s[open..].find(close_pat) {
                Some(rel_close) => {
                    let inner = s[open + open_len..open + rel_close].trim();
                    let inner = inner.strip_prefix('=').unwrap_or(inner).trim();
                    let cost_amt =
                        parse_amount_ctx(inner, &self.amount_ctx).map_err(|_| {
                            ParseError::Syntax {
                                line: line_number,
                                message: format!("invalid lot price: {}", inner),
                            }
                        })?;
                    let remaining = format!(
                        "{} {}",
                        &s[..open],
                        &s[open + rel_close + close_pat.len()..]
                    );
                    let cost = if total {
                        Cost::TotalCost(CostAmount {
                            quantity: cost_amt.quantity,
                            commodity: cost_amt.commodity,
                            style: cost_amt.style,
                        })
                    } else {
                        Cost::UnitCost(CostAmount {
                            quantity: cost_amt.quantity,
                            commodity: cost_amt.commodity,
                            style: cost_amt.style,
                        })
                    };
                    (remaining, Some(cost))
                }
                None => {
                    return Err(ParseError::Syntax {
                        line: line_number,
                        message: format!("unclosed lot price braces: {}", s),
                    })
                }
            }
        } else {
            (s.to_string(), None)
        };

        // @ / @@ cost in what remains.
        if let Some(pos) = without_lot.find("@@") {
            let amt = without_lot[..pos].trim().to_string();
            let cost_str = without_lot[pos + 2..].trim();
            let cost_amt = parse_amount_ctx(cost_str, &self.amount_ctx).map_err(|_| {
                ParseError::Syntax {
                    line: line_number,
                    message: format!("invalid cost amount: {}", cost_str),
                }
            })?;
            return Ok((
                amt,
                Some(Cost::TotalCost(CostAmount {
                    quantity: cost_amt.quantity,
                    commodity: cost_amt.commodity,
                    style: cost_amt.style,
                })),
            ));
        }
        if let Some(pos) = without_lot.find('@') {
            let amt = without_lot[..pos].trim().to_string();
            let cost_str = without_lot[pos + 1..].trim();
            let cost_amt = parse_amount_ctx(cost_str, &self.amount_ctx).map_err(|_| {
                ParseError::Syntax {
                    line: line_number,
                    message: format!("invalid cost amount: {}", cost_str),
                }
            })?;
            return Ok((
                amt,
                Some(Cost::UnitCost(CostAmount {
                    quantity: cost_amt.quantity,
                    commodity: cost_amt.commodity,
                    style: cost_amt.style,
                })),
            ));
        }

        Ok((without_lot.trim().to_string(), lot_cost))
    }

    fn parse_price_directive(&mut self, line: &str) -> Option<PriceDirective> {
        // P DATE [TIME] COMMODITY PRICE
        let rest = line.strip_prefix("P ")?.trim();
        let (date_str, rest) = split_first_word(rest);
        let date = parse_date_with_year(date_str, self.default_year).ok()?;
        let rest = rest.trim();

        // Skip optional time component (HH:MM[:SS]).
        let rest = if rest.starts_with(|c: char| c.is_ascii_digit()) && rest.contains(':') {
            let (maybe_time, after) = split_first_word(rest);
            if maybe_time.contains(':') {
                after.trim()
            } else {
                rest
            }
        } else {
            rest
        };

        // Commodity, possibly quoted.
        let (commodity_str, price_str) = if let Some(q) = rest.strip_prefix('"') {
            let close = q.find('"')?;
            (&q[..close], q[close + 1..].trim())
        } else {
            let (c, p) = split_first_word(rest);
            (c, p.trim())
        };

        // A directive may carry a trailing comment. Without stripping it the
        // amount fails to parse and the whole price is dropped in silence --
        // a journal recording "P 2021-03-08 VLXVX 27.97 USD ; from statement"
        // lost that price and every report valued from it. The commodity has
        // already been taken above, so a ';' here can only start a comment.
        let price_str = match price_str.find(';') {
            Some(i) => price_str[..i].trim(),
            None => price_str,
        };

        if price_str.is_empty() {
            return None;
        }

        let price = parse_amount_ctx(price_str, &self.amount_ctx).ok()?;

        Some(PriceDirective {
            date,
            commodity: commodity_str.to_string(),
            price_quantity: price.quantity,
            price_commodity: price.commodity,
        })
    }
}

/// Cut a trailing inline comment off a directive's argument.
///
/// hledger lets any directive carry a `; ...` note. Where the argument is
/// consumed as a value -- a path, an account name, a year -- leaving the
/// comment attached silently corrupts it: `alias Chk = Assets:Checking ; note`
/// aliased to an account literally named "Assets:Checking   ; note", and
/// `D $1,000.00 ; note` set no default commodity at all.
///
/// Not used for `account`, whose comment carries the `type:` tag and must
/// reach the classifier intact.
fn strip_inline_comment(s: &str) -> &str {
    match s.find(';') {
        Some(i) => s[..i].trim_end(),
        None => s,
    }
}

/// Append a comment line to an optional multi-line comment.
fn append_comment(slot: &mut Option<Comment>, text: &str) {
    match slot {
        Some(c) => {
            c.text.push('\n');
            c.text.push_str(text);
        }
        None => {
            *slot = Some(Comment {
                text: text.to_string(),
            });
        }
    }
}

/// Interpret posting metadata tags: `date:` / `date2:` posting dates.
fn apply_posting_meta(posting: &mut Posting, tags: &[Tag], txn_year: i32) {
    let default_year = if txn_year > 0 { Some(txn_year) } else { None };
    for tag in tags {
        let Some(value) = &tag.value else { continue };
        match tag.name.as_str() {
            "date" => {
                if let Ok(d) = parse_date_with_year(value, default_year) {
                    posting.date = Some(d);
                }
            }
            "date2" => {
                if let Ok(d) = parse_date_with_year(value, default_year) {
                    posting.date2 = Some(d);
                }
            }
            _ => {}
        }
    }
}

/// Find a char outside braces, brackets, and double quotes.
fn find_outside_delims(s: &str, needle: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_quotes = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            '{' | '[' if !in_quotes => depth += 1,
            '}' | ']' if !in_quotes => depth -= 1,
            c if c == needle && depth == 0 && !in_quotes => return Some(i),
            _ => {}
        }
    }
    None
}

/// Split account name from amount using the two-space rule.
fn split_account_amount(s: &str) -> (&str, &str) {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'\t' {
            return (&s[..i], s[i + 1..].trim_start());
        }
        if i + 1 < bytes.len() && bytes[i] == b' ' && bytes[i + 1] == b' ' {
            return (&s[..i], s[i..].trim_start());
        }
    }
    (s, "")
}

/// Split inline comment from text (a ';' outside double quotes).
fn split_inline_comment(s: &str) -> (&str, Option<Comment>) {
    match find_outside_delims(s, ';') {
        Some(pos) => {
            let before = &s[..pos];
            let comment_text = s[pos + 1..].trim();
            (
                before,
                Some(Comment {
                    text: comment_text.to_string(),
                }),
            )
        }
        None => (s, None),
    }
}

/// Parse tags from comment text, following hledger's rule: a tag name is the
/// maximal run of non-whitespace, non-comma, non-colon characters immediately
/// before a ':'; its value runs to the next comma or end of the comment.
pub(crate) fn parse_tags(comment: &str) -> Vec<Tag> {
    let mut tags = Vec::new();
    let chars: Vec<(usize, char)> = comment.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].1 == ':' {
            // Walk back to find the tag name start.
            let mut start = i;
            while start > 0 {
                let c = chars[start - 1].1;
                if c.is_whitespace() || c == ',' || c == ':' {
                    break;
                }
                start -= 1;
            }
            if start < i {
                let name: String = chars[start..i].iter().map(|(_, c)| c).collect();
                // Value: everything to the next comma.
                let mut vend = i + 1;
                while vend < chars.len() && chars[vend].1 != ',' {
                    vend += 1;
                }
                let value: String = chars[i + 1..vend]
                    .iter()
                    .map(|(_, c)| c)
                    .collect::<String>()
                    .trim()
                    .to_string();
                tags.push(Tag {
                    name,
                    value: if value.is_empty() { None } else { Some(value) },
                });
                i = vend + 1;
                continue;
            }
        }
        i += 1;
    }
    tags
}

/// Split at the first whitespace character (char-boundary safe: the previous
/// version panicked on multi-byte whitespace like NBSP).
fn split_first_word(s: &str) -> (&str, &str) {
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            return (&s[..i], &s[i + c.len_utf8()..]);
        }
    }
    (s, "")
}

/// Split a periodic-transaction header at the double-space (or tab) that
/// separates the period expression from the description.
fn split_double_space(s: &str) -> (&str, &str) {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'\t' {
            return (&s[..i], &s[i + 1..]);
        }
        if i + 1 < bytes.len() && bytes[i] == b' ' && bytes[i + 1] == b' ' {
            return (&s[..i], &s[i..]);
        }
    }
    (s, "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    fn first_txn(journal: &Journal) -> &Transaction {
        journal
            .items
            .iter()
            .find_map(|i| match i {
                JournalItem::Transaction(t) => Some(t),
                _ => None,
            })
            .expect("expected a transaction")
    }

    fn txn_count(journal: &Journal) -> usize {
        journal
            .items
            .iter()
            .filter(|i| matches!(i, JournalItem::Transaction(_)))
            .count()
    }

    #[test]
    fn parse_empty_journal() {
        let journal = parse("").unwrap();
        assert!(journal.items.is_empty());
        assert!(journal.warnings.is_empty());
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
        let txn = first_txn(&journal);

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
        assert!(journal.warnings.is_empty());
    }

    #[test]
    fn parse_transaction_with_status() {
        let input = "2024-01-15 * Cleared transaction\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let txn = first_txn(&journal);
        assert_eq!(txn.status, Status::Cleared);
        assert_eq!(txn.description, "Cleared transaction");
    }

    #[test]
    fn parse_pending_transaction() {
        let input = "2024-01-15 ! Pending transaction\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        assert_eq!(first_txn(&journal).status, Status::Pending);
    }

    #[test]
    fn parse_transaction_with_code() {
        let input = "2024-01-15 (1234) Payee\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let txn = first_txn(&journal);
        assert_eq!(txn.code, Some("1234".to_string()));
        assert_eq!(txn.description, "Payee");
    }

    #[test]
    fn parse_multicurrency_transaction() {
        let input = "2024-01-15 Exchange\n    assets:eur  100.00 EUR\n    assets:usd  -110.00 USD\n";
        let journal = parse(input).unwrap();
        let txn = first_txn(&journal);
        assert_eq!(txn.postings.len(), 2);
        assert_eq!(txn.postings[0].amount.as_ref().unwrap().quantity, dec!(100.00));
        assert_eq!(txn.postings[1].amount.as_ref().unwrap().quantity, dec!(-110.00));
    }

    #[test]
    fn parse_transaction_with_cost() {
        let input = "2024-01-15 Exchange\n    assets:eur  100.00 EUR @ $1.10\n    assets:usd\n";
        let journal = parse(input).unwrap();
        let amt = first_txn(&journal).postings[0].amount.as_ref().unwrap();
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
        let input = "2024-01-15 Exchange\n    assets:eur  100.00 EUR @@ $110.00\n    assets:usd\n";
        let journal = parse(input).unwrap();
        let amt = first_txn(&journal).postings[0].amount.as_ref().unwrap();
        match &amt.cost {
            Some(Cost::TotalCost(c)) => {
                assert_eq!(c.quantity, dec!(110.00));
                assert_eq!(c.commodity, "$");
            }
            other => panic!("expected total cost, got {:?}", other),
        }
    }

    #[test]
    fn lot_price_with_at_cost_prefers_at() {
        // hledger 1.32: when both {} and @ appear, @ is the cost.
        let input = "2024-01-15 Buy\n    assets:stock  2 AAPL {$4.00} @ $5.00\n    assets:cash\n";
        let journal = parse(input).unwrap();
        let amt = first_txn(&journal).postings[0].amount.as_ref().unwrap();
        match &amt.cost {
            Some(Cost::UnitCost(c)) => assert_eq!(c.quantity, dec!(5.00)),
            other => panic!("expected unit cost 5.00, got {:?}", other),
        }
    }

    #[test]
    fn lot_price_alone_used_as_cost() {
        let input = "2024-01-15 Sell\n    assets:stock  -19 ITOT {96.15 USD}\n    assets:cash  1826.85 USD\n";
        let journal = parse(input).unwrap();
        let amt = first_txn(&journal).postings[0].amount.as_ref().unwrap();
        match &amt.cost {
            Some(Cost::UnitCost(c)) => assert_eq!(c.quantity, dec!(96.15)),
            other => panic!("expected unit cost, got {:?}", other),
        }
    }

    #[test]
    fn parse_transaction_with_inline_comment() {
        let input =
            "2024-01-15 Grocery ; category:food\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let txn = first_txn(&journal);
        assert_eq!(txn.description, "Grocery");
        assert!(txn.comment.is_some());
        assert_eq!(txn.tags.len(), 1);
        assert_eq!(txn.tags[0].name, "category");
        assert_eq!(txn.tags[0].value, Some("food".to_string()));
    }

    #[test]
    fn parse_body_comment_lines_preserved() {
        let input = "2024-01-15 Grocery\n    ; txntag:tval\n    expenses:food  $50.00\n    ; posting note\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let txn = first_txn(&journal);
        // Comment before first posting attaches to the transaction.
        assert_eq!(txn.comment.as_ref().unwrap().text, "txntag:tval");
        assert_eq!(txn.tags[0].name, "txntag");
        // Comment after a posting attaches to that posting.
        assert_eq!(
            txn.postings[0].comment.as_ref().unwrap().text,
            "posting note"
        );
    }

    #[test]
    fn parse_posting_date_tag() {
        let input = "2024-01-15 T\n    expenses:food  $5.00 ; date:2024-01-20\n    assets:cash\n";
        let journal = parse(input).unwrap();
        let txn = first_txn(&journal);
        assert_eq!(
            txn.postings[0].date,
            Some(NaiveDate::from_ymd_opt(2024, 1, 20).unwrap())
        );
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
        assert_eq!(txn_count(&journal), 2);
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
    fn parse_price_directive_quoted_commodity() {
        let input = "P 2024-01-15 \"MY FUND\" $10.00\n";
        let journal = parse(input).unwrap();
        match &journal.items[0] {
            JournalItem::PriceDirective(pd) => assert_eq!(pd.commodity, "MY FUND"),
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
    fn parse_account_type_declaration() {
        let input = "account aktiva:bank  ; type:A\naccount other\n    ; type:L\n";
        let journal = parse(input).unwrap();
        match &journal.items[0] {
            JournalItem::AccountDirective(ad) => {
                assert_eq!(ad.tags[0].name, "type");
                assert_eq!(ad.tags[0].value, Some("A".to_string()));
            }
            _ => panic!("expected account directive"),
        }
        match &journal.items[1] {
            JournalItem::AccountDirective(ad) => {
                assert_eq!(ad.tags[0].value, Some("L".to_string()));
            }
            _ => panic!("expected account directive"),
        }
    }

    #[test]
    fn parse_date_formats() {
        for sep in &["-", "/", "."] {
            let input = format!(
                "2024{}01{}15 Test\n    expenses:food  $50.00\n    assets:checking\n",
                sep, sep
            );
            let journal = parse(&input).unwrap();
            assert_eq!(
                first_txn(&journal).date,
                NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
            );
        }
    }

    #[test]
    fn parse_posting_with_balance_assertion() {
        let input = "2024-01-15 Opening\n    assets:checking  $1000 = $1000\n    equity:opening\n";
        let journal = parse(input).unwrap();
        let p = &first_txn(&journal).postings[0];
        assert_eq!(p.amount.as_ref().unwrap().quantity, dec!(1000));
        let assertion = p.balance_assertion.as_ref().unwrap();
        assert!(!assertion.strong);
        assert!(!assertion.inclusive);
        assert_eq!(assertion.quantity, dec!(1000));
        assert_eq!(assertion.commodity, "$");
    }

    #[test]
    fn parse_inclusive_assertions() {
        let input = "2024-01-15 T\n    assets  $10 =* $160\n    equity\n";
        let journal = parse(input).unwrap();
        let a = first_txn(&journal).postings[0]
            .balance_assertion
            .as_ref()
            .unwrap();
        assert!(a.inclusive);
        assert!(!a.strong);

        let input = "2024-01-15 T\n    assets  $10 ==* $160\n    equity\n";
        let journal = parse(input).unwrap();
        let a = first_txn(&journal).postings[0]
            .balance_assertion
            .as_ref()
            .unwrap();
        assert!(a.inclusive);
        assert!(a.strong);
    }

    #[test]
    fn parse_balance_assignment() {
        let input = "2024-01-15 T\n    assets:bank  = $500\n    equity:opening\n";
        let journal = parse(input).unwrap();
        let p = &first_txn(&journal).postings[0];
        assert!(p.amount.is_none());
        assert_eq!(p.balance_assertion.as_ref().unwrap().quantity, dec!(500));
    }

    #[test]
    fn parse_secondary_date() {
        let input = "2024-01-15=2024-01-16 Test\n    expenses:food  $50.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        let txn = first_txn(&journal);
        assert_eq!(txn.date, NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
        assert_eq!(
            txn.secondary_date,
            Some(NaiveDate::from_ymd_opt(2024, 1, 16).unwrap())
        );
    }

    #[test]
    fn parse_secondary_date_partial() {
        let input = "2024-01-15=01-16 Test\n    e  $1\n    a\n";
        let journal = parse(input).unwrap();
        assert_eq!(
            first_txn(&journal).secondary_date,
            Some(NaiveDate::from_ymd_opt(2024, 1, 16).unwrap())
        );
    }

    #[test]
    fn comment_block_is_inert() {
        let input = "comment\n2024-01-01 Phantom\n    expenses:x  $999.00\n    assets:y\nend comment\n2024-01-02 Real\n    expenses:a  $10.00\n    assets:b\n";
        let journal = parse(input).unwrap();
        assert_eq!(txn_count(&journal), 1);
        assert_eq!(first_txn(&journal).description, "Real");
    }

    #[test]
    fn year_directive_enables_partial_dates() {
        let input = "Y 2024\n01-15 Partial\n    expenses:a  $50.00\n    assets:b\n";
        let journal = parse(input).unwrap();
        assert_eq!(txn_count(&journal), 1);
        assert_eq!(
            first_txn(&journal).date,
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
    }

    #[test]
    fn nbsp_after_date_does_not_panic() {
        let input = "2024-01-15\u{a0}Payee\n    expenses:a  $5.00\n    assets:b\n";
        let journal = parse(input).unwrap();
        assert_eq!(first_txn(&journal).description, "Payee");
    }

    #[test]
    fn crlf_spans_are_byte_accurate() {
        let input = "2024-01-01 First\r\n    a  $1.00\r\n    b\r\n\r\n2024-01-16 Second\r\n    a  $2.00\r\n    b\r\n";
        let journal = parse(input).unwrap();
        let txns: Vec<&Transaction> = journal
            .items
            .iter()
            .filter_map(|i| match i {
                JournalItem::Transaction(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(txns.len(), 2);
        let t2 = &input[txns[1].span.start..txns[1].span.end];
        assert!(t2.starts_with("2024-01-16 Second"), "span was {:?}", t2);
        let t1 = &input[txns[0].span.start..txns[0].span.end];
        assert!(t1.contains("    b"), "span was {:?}", t1);
    }

    #[test]
    fn unknown_line_produces_warning() {
        let input = "2024-01-15 Ok\n    a  $1\n    b\n\nAssets:Oops  $5.00\n";
        let journal = parse(input).unwrap();
        assert_eq!(txn_count(&journal), 1);
        assert_eq!(journal.warnings.len(), 1);
        assert!(journal.warnings[0].message.contains("Assets:Oops"));
    }

    #[test]
    fn periodic_full_period_expression() {
        let input = "~ every 2 weeks from 2024-01  Groceries budget\n    expenses:food  $100.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        match &journal.items[0] {
            JournalItem::PeriodicTransaction(pt) => {
                assert_eq!(pt.period, "every 2 weeks from 2024-01");
                assert_eq!(pt.description, "Groceries budget");
                assert_eq!(pt.postings.len(), 2);
            }
            _ => panic!("expected periodic transaction"),
        }
    }

    #[test]
    fn periodic_no_description() {
        let input = "~ monthly from 2024-02\n    expenses:rent  $1000.00\n    assets:checking\n";
        let journal = parse(input).unwrap();
        match &journal.items[0] {
            JournalItem::PeriodicTransaction(pt) => {
                assert_eq!(pt.period, "monthly from 2024-02");
                assert_eq!(pt.description, "");
            }
            _ => panic!("expected periodic transaction"),
        }
    }

    #[test]
    fn auto_posting_multiplier() {
        let input = "= expenses:food\n    (budget:food)  *2\n    (budget:reserve)  *0.5\n";
        let journal = parse(input).unwrap();
        match &journal.items[0] {
            JournalItem::AutoPostingRule(rule) => {
                assert_eq!(rule.postings.len(), 2);
                assert!(rule.postings[0].amount.as_ref().unwrap().multiplier);
                assert_eq!(rule.postings[0].amount.as_ref().unwrap().quantity, dec!(2));
                assert_eq!(
                    rule.postings[1].amount.as_ref().unwrap().quantity,
                    dec!(0.5)
                );
            }
            _ => panic!("expected auto posting rule"),
        }
    }

    #[test]
    fn decimal_mark_directive_applies() {
        let input = "decimal-mark ,\n2024-01-15 T\n    expenses:a  1.234,56 EUR\n    assets:b  -1.234,56 EUR\n";
        let journal = parse(input).unwrap();
        assert_eq!(
            first_txn(&journal).postings[0].amount.as_ref().unwrap().quantity,
            dec!(1234.56)
        );
    }

    #[test]
    fn commodity_format_applies() {
        let input = "commodity 1.000,00 EUR\n2024-01-15 T\n    expenses:a  1.234 EUR\n    assets:b  -1.234 EUR\n";
        let journal = parse(input).unwrap();
        // EUR uses ',' decimal → lone '.' is grouping.
        assert_eq!(
            first_txn(&journal).postings[0].amount.as_ref().unwrap().quantity,
            dec!(1234)
        );
    }

    #[test]
    fn default_commodity_directive() {
        let input = "D $1,000.00\n2024-01-15 T\n    expenses:a  25\n    assets:b\n";
        let journal = parse(input).unwrap();
        let amt = first_txn(&journal).postings[0].amount.as_ref().unwrap();
        assert_eq!(amt.commodity, "$");
        assert_eq!(amt.quantity, dec!(25));
    }

    #[test]
    fn apply_account_prefixes() {
        let input = "apply account personal\n2024-01-15 T\n    expenses:food  $1\n    assets:cash\nend apply account\n2024-01-16 T2\n    expenses:food  $1\n    assets:cash\n";
        let journal = parse(input).unwrap();
        let txns: Vec<&Transaction> = journal
            .items
            .iter()
            .filter_map(|i| match i {
                JournalItem::Transaction(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(txns[0].postings[0].account.full, "personal:expenses:food");
        assert_eq!(txns[1].postings[0].account.full, "expenses:food");
    }

    #[test]
    fn simple_alias_applied() {
        let input = "alias food = expenses:food\n2024-01-15 T\n    food:snacks  $1\n    assets:cash\n";
        let journal = parse(input).unwrap();
        assert_eq!(
            first_txn(&journal).postings[0].account.full,
            "expenses:food:snacks"
        );
    }

    #[test]
    fn regex_alias_applied() {
        let input = "alias /^food/ = expenses:food\n2024-01-15 T\n    food  $1\n    assets:cash\n";
        let journal = parse(input).unwrap();
        assert_eq!(first_txn(&journal).postings[0].account.full, "expenses:food");
    }

    #[test]
    fn tags_last_word_before_colon() {
        let tags = parse_tags("just a note: with colon");
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "note");
        assert_eq!(tags[0].value, Some("with colon".to_string()));

        let tags = parse_tags("a:1, b:2");
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].name, "a");
        assert_eq!(tags[1].name, "b");
        assert_eq!(tags[1].value, Some("2".to_string()));
    }

    #[test]
    fn quoted_commodity_semicolon_not_comment() {
        let input = "2024-01-15 T\n    assets:funds  1.5 \"FUND;X\"\n    assets:cash  -1.5 \"FUND;X\"\n";
        let journal = parse(input).unwrap();
        assert_eq!(
            first_txn(&journal).postings[0].amount.as_ref().unwrap().commodity,
            "FUND;X"
        );
    }

    #[test]
    fn bad_digit_line_is_hard_error() {
        let input = "12345678 not a date line\n";
        let err = parse(input).unwrap_err();
        match err {
            ParseError::Syntax { line, .. } => assert_eq!(line, 1),
            other => panic!("expected syntax error with line, got {:?}", other),
        }
    }

    #[test]
    fn empty_account_name_is_error() {
        // A virtual posting with an empty account: "()  $5.00"
        let input = "2024-01-15 T\n    ()  $5.00\n    assets:cash\n";
        assert!(parse(input).is_err());
    }
}

#[cfg(test)]
mod price_comment_tests {
    use super::*;

    /// Every directive whose argument is consumed as a value must survive a
    /// trailing comment. hledger permits one on any directive, and leaving it
    /// attached corrupts the value silently -- an alias pointing at an account
    /// name with "; note" welded on, or a default commodity that vanishes.
    #[test]
    fn directives_may_carry_a_trailing_comment() {
        fn first_txn(j: &Journal) -> &Transaction {
            j.items
                .iter()
                .find_map(|i| match i {
                    JournalItem::Transaction(t) => Some(t),
                    _ => None,
                })
                .expect("expected a transaction")
        }

        // `year` sets the default year for partial dates.
        let j = parse("year 2024   ; fiscal\n\n01-05 x\n    a   1 FOO\n    b\n").unwrap();
        let dated: Vec<_> = j
            .items
            .iter()
            .filter_map(|i| match i {
                JournalItem::Transaction(t) => Some(t.date.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(dated, vec!["2024-01-05"]);

        // `alias` rewrites an account name.
        let j = parse("alias Chk = Assets:Checking   ; note\n\n2024-01-01 x\n    Chk   1 FOO\n    b\n").unwrap();
        assert_eq!(first_txn(&j).postings[0].account.full, "Assets:Checking");

        // `D` sets the default commodity for bare amounts.
        let j = parse("D $1,000.00   ; note\n\n2024-01-01 x\n    a   5\n    b\n").unwrap();
        assert_eq!(
            first_txn(&j).postings[0].amount.as_ref().unwrap().commodity,
            "$"
        );

        // `include` names a path.
        let j = parse("include other.journal   ; pulled in\n").unwrap();
        let inc = j
            .items
            .iter()
            .find_map(|i| match i {
                JournalItem::IncludeDirective(d) => Some(d),
                _ => None,
            })
            .expect("expected an include");
        assert_eq!(inc.path, "other.journal");

        // `apply account` prefixes every account below it.
        let j = parse("apply account Assets   ; note\n\n2024-01-01 x\n    Bank   1 FOO\n    b\n").unwrap();
        assert_eq!(first_txn(&j).postings[0].account.full, "Assets:Bank");
    }

    /// The `account` directive is the exception: its comment carries the
    /// `type:` tag, so it must reach the classifier intact.
    #[test]
    fn an_account_directive_keeps_its_comment() {
        let j = parse("account Assets:B  ; type:C, spending money\n").unwrap();
        // Classification itself is covered in hledger-core; here we only
        // assert the comment was not discarded on the way through.
        let text = format!("{:?}", j.items[0]);
        assert!(text.contains("type:C"), "type tag lost: {text}");
    }

    /// A trailing comment on a price directive must not discard the price.
    ///
    /// A real journal annotating its prices ("; from statement/trade") lost 21
    /// of 137 prices this way, silently, which then skewed every valued
    /// report -- net worth, balance sheet, returns.
    #[test]
    fn a_price_directive_may_carry_a_comment() {
        let journal = parse(concat!(
            "P 2021-03-08 VLXVX 27.97 USD   ; from statement/trade\n",
            "P 2021-03-09 VLXVX 28.10 USD\n",
        ))
        .unwrap();
        let prices: Vec<_> = journal
            .items
            .iter()
            .filter_map(|i| match i {
                JournalItem::PriceDirective(pd) => Some(pd),
                _ => None,
            })
            .collect();
        assert_eq!(prices.len(), 2, "both prices must survive");
        assert_eq!(prices[0].commodity, "VLXVX");
        assert_eq!(prices[0].price_commodity, "USD");
        assert_eq!(prices[0].price_quantity.to_string(), "27.97");
    }
}
