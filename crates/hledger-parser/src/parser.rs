use std::collections::BTreeMap;

use chrono::Datelike;

use crate::amount::{parse_amount_ctx, parse_style_example, AmountContext};
use crate::ast::*;
use crate::date::parse_date_with_year;
use crate::error::ParseError;

/// Parse a journal file from a string, with no inherited directives.
pub fn parse(input: &str) -> Result<Journal, ParseError> {
    parse_with_context(input, &ParseContext::default())
}

/// Parse a journal file that was `include`d from another: `ctx` carries the
/// directives in effect at the include point (hledger semantics: `D`, `Y`,
/// `alias`, `apply account`, `decimal-mark` and `commodity` all flow into the
/// included file).
pub fn parse_with_context(input: &str, ctx: &ParseContext) -> Result<Journal, ParseError> {
    parse_with_context_result(input, ctx).map(|(journal, _)| journal)
}

/// Like [`parse_with_context`], also returning the context as it stands at
/// the end of the file. The caller that follows `include` directives uses it
/// two ways:
///
/// * pass a clone of the *current* context into each included file (it sees
///   everything declared before the `include` line);
/// * after the include returns, fold the child's global declarations back
///   with [`ParseContext::absorb_global`] — `commodity` directives are
///   journal-wide in hledger, while `D`, `Y`, `alias`, `apply account` and
///   `decimal-mark` stay scoped to the file that set them.
pub fn parse_with_context_result(
    input: &str,
    ctx: &ParseContext,
) -> Result<(Journal, ParseContext), ParseError> {
    let parsed = parse_file_with_context(input, ctx)?;
    Ok((parsed.journal, parsed.context))
}

/// One parsed file plus the directive state it produced.
#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub journal: Journal,
    /// State at the end of the file.
    pub context: ParseContext,
    /// State at each `include` directive, in order of appearance. An
    /// included file inherits the state in force *where it is included*
    /// (an `apply account` closed later in the file still applies to it),
    /// so the loader must not use the end-of-file context for that.
    pub include_contexts: Vec<ParseContext>,
}

/// Parse a file with inherited directive state; see [`ParsedFile`].
pub fn parse_file_with_context(input: &str, ctx: &ParseContext) -> Result<ParsedFile, ParseError> {
    Parser::new(input, ctx.clone()).run()
}

/// Directive state that flows between files and out to the application.
///
/// Everything the parser learns from directives lives here so that included
/// files can inherit it and so the application can format amounts the way
/// the journal writes them (see [`ParseContext::style_for`]).
#[derive(Debug, Clone, Default)]
pub struct ParseContext {
    /// Number-format context: `decimal-mark`, per-commodity decimal marks
    /// (from `commodity`/`D`), and the `D` default commodity.
    pub amount_ctx: AmountContext,
    /// Aliases in effect, in declaration order (`end aliases` clears them).
    pub aliases: Vec<AliasDirective>,
    /// Open `apply account` prefixes, outermost first.
    pub account_prefix: Vec<String>,
    /// `Y`/`year` default year for partial dates.
    pub default_year: Option<i32>,
    /// Styles declared by `commodity` directives (journal-wide in hledger).
    pub declared_styles: BTreeMap<String, AmountStyle>,
    /// Styles observed on posting amounts, merged per commodity the way
    /// hledger builds a canonical style: first-seen side/spacing/mark, the
    /// highest precision, and the first digit grouping seen.
    pub observed_styles: BTreeMap<String, AmountStyle>,
}

impl ParseContext {
    /// Fold the journal-wide parts of a child (included) file's resulting
    /// context back into this one. `commodity` declarations and observed
    /// amount styles are global; everything else stays with the child.
    pub fn absorb_global(&mut self, child: &ParseContext) {
        for (commodity, style) in &child.declared_styles {
            self.declared_styles.insert(commodity.clone(), style.clone());
            self.amount_ctx
                .commodity_marks
                .insert(commodity.clone(), style.decimal_mark);
        }
        for (commodity, style) in &child.observed_styles {
            self.observed_styles
                .entry(commodity.clone())
                .and_modify(|existing| existing.absorb(style))
                .or_insert_with(|| style.clone());
        }
    }

    /// The style a commodity is written in: a `commodity` directive's format
    /// if declared, otherwise the merged style of the amounts seen (the `D`
    /// directive's style for its commodity when nothing else is known).
    /// `None` for a commodity the journal has never written.
    pub fn style_for(&self, commodity: &str) -> Option<AmountStyle> {
        if let Some(style) = self.declared_styles.get(commodity) {
            return Some(style.clone());
        }
        if let Some(style) = self.observed_styles.get(commodity) {
            return Some(style.clone());
        }
        match &self.amount_ctx.default_commodity {
            Some((c, style)) if c == commodity => Some(style.clone()),
            _ => None,
        }
    }

    /// The number-format context, for parsing amounts typed by the user the
    /// way the journal would read them (`decimal-mark`, `D`, commodity marks).
    pub fn amount_context(&self) -> &AmountContext {
        &self.amount_ctx
    }

    /// Every commodity with a known style, declared or observed.
    pub fn known_commodities(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .declared_styles
            .keys()
            .chain(self.observed_styles.keys())
            .cloned()
            .collect();
        if let Some((c, _)) = &self.amount_ctx.default_commodity {
            out.push(c.clone());
        }
        out.sort();
        out.dedup();
        out
    }
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

fn compile_alias(directive: &AliasDirective) -> Option<CompiledAlias> {
    let matcher = if directive.regex {
        AliasMatcher::Regex(regex::Regex::new(&directive.from).ok()?)
    } else {
        AliasMatcher::Prefix(directive.from.clone())
    };
    Some(CompiledAlias {
        to: directive.to.clone(),
        matcher,
    })
}

struct Parser<'a> {
    input: &'a str,
    /// Lines with any trailing '\r' stripped.
    lines: Vec<&'a str>,
    /// Byte offset of each line's start in the ORIGINAL input (CRLF-safe).
    line_starts: Vec<usize>,
    items: Vec<JournalItem>,
    warnings: Vec<ParseWarning>,

    // Parse state from directives: inherited from the including file, and
    // handed back at the end so the caller can propagate it.
    ctx: ParseContext,
    aliases: Vec<CompiledAlias>,
    /// Snapshot of `ctx` at each `include` directive, in order.
    include_contexts: Vec<ParseContext>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, ctx: ParseContext) -> Self {
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

        let aliases = ctx.aliases.iter().filter_map(compile_alias).collect();
        Self {
            input,
            lines,
            line_starts,
            items: Vec::new(),
            warnings: Vec::new(),
            ctx,
            aliases,
            include_contexts: Vec::new(),
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

    fn run(mut self) -> Result<ParsedFile, ParseError> {
        if self.input.trim().is_empty() {
            return Ok(ParsedFile {
                journal: Journal {
                    items: vec![],
                    source_path: None,
                    warnings: vec![],
                },
                context: self.ctx,
                include_contexts: vec![],
            });
        }

        let mut i = 0;
        while i < self.lines.len() {
            i = self.parse_item(i)?;
        }

        Ok(ParsedFile {
            journal: Journal {
                items: self.items,
                source_path: None,
                warnings: self.warnings,
            },
            context: self.ctx,
            include_contexts: self.include_contexts,
        })
    }

    /// Remember how an amount was written, so the journal's style for its
    /// commodity can be reproduced later.
    fn observe_style(&mut self, commodity: &str, style: &AmountStyle) {
        self.ctx
            .observed_styles
            .entry(commodity.to_string())
            .and_modify(|existing| existing.absorb(style))
            .or_insert_with(|| style.clone());
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
            // A price directive says nothing about how the priced commodity's
            // numbers are written: forcing '.' here made `3,50 EUR` read as
            // 350 EUR after a `P ... EUR` line.
            match self.parse_price_directive(line) {
                Some(pd) => {
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
            self.include_contexts.push(self.ctx.clone());
            return Ok(Some(i + 1));
        }
        if let Some(rest) = line.strip_prefix("alias ") {
            self.parse_alias_directive(strip_inline_comment(rest), i);
            return Ok(Some(i + 1));
        }
        if line.trim() == "end aliases" {
            self.aliases.clear();
            self.ctx.aliases.clear();
            self.items.push(JournalItem::OtherDirective(line.to_string()));
            return Ok(Some(i + 1));
        }
        if let Some(rest) = line.strip_prefix("decimal-mark ") {
            if let Some(ch) = rest.trim().chars().next() {
                self.ctx.amount_ctx.decimal_mark = Some(ch);
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
                    self.ctx
                        .amount_ctx
                        .commodity_marks
                        .insert(commodity.clone(), style.decimal_mark);
                    self.ctx.amount_ctx.default_commodity = Some((commodity, style));
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
                Ok(y) => self.ctx.default_year = Some(y),
                Err(_) => self.warn(i + 1, format!("malformed year directive: {}", line)),
            }
            self.items.push(JournalItem::OtherDirective(line.to_string()));
            return Ok(Some(i + 1));
        }
        if let Some(rest) = line.strip_prefix("apply account ") {
            self.ctx
                .account_prefix
                .push(strip_inline_comment(rest).trim().to_string());
            self.items.push(JournalItem::OtherDirective(line.to_string()));
            return Ok(Some(i + 1));
        }
        if line.trim() == "end apply account" {
            if self.ctx.account_prefix.pop().is_none() {
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
        // hledger applies the current `apply account` prefix and aliases to
        // declared names too (`modifiedaccountnamep`), so `alias /^foo/ =
        // bar` followed by `account foo:x` declares `bar:x`.
        let name = self.resolve_account(name_part.trim());

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
            self.ctx
                .amount_ctx
                .commodity_marks
                .insert(commodity.clone(), style.decimal_mark);
            self.ctx
                .declared_styles
                .insert(commodity.clone(), style.clone());
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
                            let directive = AliasDirective {
                                from: pattern.to_string(),
                                to: repl.to_string(),
                                regex: true,
                            };
                            self.ctx.aliases.push(directive.clone());
                            self.items.push(JournalItem::AliasDirective(directive));
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
            let directive = AliasDirective { from, to, regex: false };
            self.ctx.aliases.push(directive.clone());
            self.items.push(JournalItem::AliasDirective(directive));
        } else {
            self.warn(i + 1, format!("malformed alias directive: alias {}", rest));
        }
    }

    /// Apply `apply account` prefix and aliases to an account name.
    fn resolve_account(&self, raw: &str) -> AccountName {
        let mut name = if self.ctx.account_prefix.is_empty() {
            raw.to_string()
        } else {
            format!("{}:{}", self.ctx.account_prefix.join(":"), raw)
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
                        // hledger replaces every match, not just the first.
                        name = re.replace_all(&name, alias.to.as_str()).into_owned();
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
        // hledger reads the header up to the first ';' with no quote
        // handling: a description like `Bob's "thing` must not swallow the
        // comment and its tags.
        let (header, mut comment) = split_header_comment(header);
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

        let date = parse_date_with_year(date_str, self.ctx.default_year).map_err(|e| {
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
                style: q.style(Side::Left, false),
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
                    parse_amount_ctx(amt_part, &self.ctx.amount_ctx).map_err(|_| {
                        ParseError::Syntax {
                            line: line_number,
                            message: format!("invalid amount: {}", amount_str),
                        }
                    })?;
                parsed.cost = cost;
                if !parsed.commodity.is_empty() {
                    self.observe_style(&parsed.commodity.clone(), &parsed.style.clone());
                }
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

        let amt = parse_amount_ctx(assertion_str, &self.ctx.amount_ctx).map_err(|_| {
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
                        parse_amount_ctx(inner, &self.ctx.amount_ctx).map_err(|_| {
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
            let cost_amt = parse_amount_ctx(cost_str, &self.ctx.amount_ctx).map_err(|_| {
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
            let cost_amt = parse_amount_ctx(cost_str, &self.ctx.amount_ctx).map_err(|_| {
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
        let date = parse_date_with_year(date_str, self.ctx.default_year).ok()?;
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

        let price = parse_amount_ctx(price_str, &self.ctx.amount_ctx).ok()?;

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

/// Split a transaction header at its first ';'. Unlike posting lines, the
/// header has no quoted commodities, and hledger does no quote handling
/// there.
fn split_header_comment(s: &str) -> (&str, Option<Comment>) {
    match s.find(';') {
        Some(pos) => (
            &s[..pos],
            Some(Comment {
                text: s[pos + 1..].trim().to_string(),
            }),
        ),
        None => (s, None),
    }
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
    use rust_decimal::Decimal;
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

    fn amounts(journal: &Journal) -> Vec<Decimal> {
        journal
            .items
            .iter()
            .filter_map(|i| match i {
                JournalItem::Transaction(t) => t.postings[0].amount.as_ref().map(|a| a.quantity),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn price_directive_does_not_force_a_decimal_mark() {
        // hledger 1.50.3 reads these as 3.50 and 1234.56 EUR; forcing '.'
        // after the P line made `3,50 EUR` into 350.
        let journal = parse(concat!(
            "P 2024-01-01 EUR $1.10\n\n",
            "2024-01-02 a\n    x  3,50 EUR\n    y\n\n",
            "2024-01-03 b\n    x  1.234,56 EUR\n    y\n",
        ))
        .unwrap();
        assert_eq!(amounts(&journal), vec![dec!(3.50), dec!(1234.56)]);
    }

    #[test]
    fn included_file_inherits_directives_and_commodities_flow_back() {
        // Verified against hledger 1.50.3 with an `include`: the child sees
        // the parent's D, Y, alias and apply account; the child's commodity
        // directive styles the parent's amounts.
        let (_, parent_ctx) = parse_with_context_result(
            "D 1.000,00 EUR\nY 2023\nalias foo = bar\napply account top\n",
            &ParseContext::default(),
        )
        .unwrap();
        assert_eq!(parent_ctx.default_year, Some(2023));
        assert_eq!(parent_ctx.account_prefix, vec!["top".to_string()]);

        let (child, child_ctx) = parse_with_context_result(
            "commodity 1,000.000 XYZ\n01-05 c\n    foo  1,5\n    y\n",
            &parent_ctx,
        )
        .unwrap();
        let txn = first_txn(&child);
        assert_eq!(txn.date, NaiveDate::from_ymd_opt(2023, 1, 5).unwrap());
        // hledger applies the prefix first, then aliases: `foo` becomes
        // `top:foo`, which a basic alias anchored at the start does not match.
        assert_eq!(txn.postings[0].account.full, "top:foo");
        assert_eq!(txn.postings[1].account.full, "top:y");
        let amt = txn.postings[0].amount.as_ref().unwrap();
        assert_eq!(amt.commodity, "EUR");
        assert_eq!(amt.quantity, dec!(1.5));

        // The parent folds the child's global declarations back in.
        let mut merged = parent_ctx.clone();
        merged.absorb_global(&child_ctx);
        assert_eq!(merged.style_for("XYZ").unwrap().precision, 3);
        assert_eq!(merged.amount_ctx.commodity_marks.get("XYZ"), Some(&'.'));
        // But not the child's file-scoped state.
        assert_eq!(merged.account_prefix, vec!["top".to_string()]);

        // With a decimal-mark in the parent, the child reads numbers that way.
        let (_, ctx) =
            parse_with_context_result("decimal-mark ,\n", &ParseContext::default()).unwrap();
        let child = parse_with_context("2024-01-01 c\n    a  1.234 EUR\n    b\n", &ctx).unwrap();
        assert_eq!(amounts(&child), vec![dec!(1234)]);
    }

    #[test]
    fn context_reports_the_journals_style_for_each_commodity() {
        let (_, ctx) = parse_with_context_result(
            concat!(
                "commodity 1.000,00 EUR\n\n",
                "2024-01-01 a\n    x  $1,000.5\n    y\n\n",
                "2024-01-02 b\n    x  $2.25\n    y\n\n",
                "2024-01-03 c\n    x  10 AAPL\n    y\n",
            ),
            &ParseContext::default(),
        )
        .unwrap();
        let usd = ctx.style_for("$").unwrap();
        assert_eq!(usd.commodity_side, Side::Left);
        assert_eq!(usd.precision, 2, "the highest precision seen");
        assert_eq!(usd.digit_group_mark, Some(','));
        let eur = ctx.style_for("EUR").unwrap();
        assert_eq!(eur.decimal_mark, ',');
        assert_eq!(eur.digit_group_mark, Some('.'));
        assert_eq!(ctx.style_for("AAPL").unwrap().precision, 0);
        assert!(ctx.style_for("BTC").is_none());
        assert_eq!(ctx.known_commodities(), vec!["$", "AAPL", "EUR"]);
    }

    #[test]
    fn regex_alias_replaces_every_match_and_applies_to_account_directives() {
        // hledger: `alias /^foo/ = bar` + `account foo:x` declares bar:x.
        let journal = parse("alias /o/ = 0\naccount foo:x\n2024-01-01 t\n    foo  $1\n    y\n").unwrap();
        let declared = journal
            .items
            .iter()
            .find_map(|i| match i {
                JournalItem::AccountDirective(a) => Some(a.name.full.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(declared, "f00:x");
        assert_eq!(first_txn(&journal).postings[0].account.full, "f00");
    }

    #[test]
    fn unmatched_quote_in_description_does_not_swallow_the_comment() {
        // hledger has no quote handling on the header line.
        let journal = parse("2024-01-01 Bob's \"thing ; tag:val\n    a  $1\n    b\n").unwrap();
        let txn = first_txn(&journal);
        assert_eq!(txn.description, "Bob's \"thing");
        assert_eq!(txn.tags, vec![Tag { name: "tag".into(), value: Some("val".into()) }]);

        // Quoted commodities on posting lines still protect their ';'.
        let journal = parse("2024-01-01 x\n    a  1 \"A;B\" ; note:n\n    b\n").unwrap();
        let p = &first_txn(&journal).postings[0];
        assert_eq!(p.amount.as_ref().unwrap().commodity, "A;B");
        assert_eq!(p.tags[0].name, "note");
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
