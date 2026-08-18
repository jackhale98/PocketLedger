use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Serialize;
use std::path::PathBuf;

/// Byte offset span in source text for round-trip patching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
    pub line: usize,
}

/// A non-fatal problem found while parsing. The journal still loads, but the
/// user should be told: silent divergence from hledger is worse than a warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParseWarning {
    pub line: usize,
    pub message: String,
}

/// Top-level container: everything in one journal file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Journal {
    pub items: Vec<JournalItem>,
    pub source_path: Option<PathBuf>,
    pub warnings: Vec<ParseWarning>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum JournalItem {
    Transaction(Transaction),
    Comment(Comment),
    BlankLine,
    AccountDirective(AccountDirective),
    CommodityDirective(CommodityDirective),
    PriceDirective(PriceDirective),
    IncludeDirective(IncludeDirective),
    AliasDirective(AliasDirective),
    DecimalMarkDirective(DecimalMarkDirective),
    PeriodicTransaction(PeriodicTransaction),
    AutoPostingRule(AutoPostingRule),
    /// A directive we recognize and preserve verbatim but attach no semantics
    /// to (payee, tag, apply/end markers, ledger-isms). Kept raw for round-trip.
    OtherDirective(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Transaction {
    pub span: SourceSpan,
    pub date: NaiveDate,
    pub secondary_date: Option<NaiveDate>,
    pub status: Status,
    pub code: Option<String>,
    pub description: String,
    /// Inline comment plus any following indented comment lines, joined with '\n'.
    pub comment: Option<Comment>,
    pub tags: Vec<Tag>,
    pub postings: Vec<Posting>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    Unmarked,
    Pending,
    Cleared,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Posting {
    pub span: SourceSpan,
    pub status: Status,
    pub account: AccountName,
    pub amount: Option<PostingAmount>,
    pub balance_assertion: Option<BalanceAssertion>,
    /// Inline comment plus any following indented comment lines, joined with '\n'.
    pub comment: Option<Comment>,
    pub tags: Vec<Tag>,
    pub is_virtual: bool,
    pub virtual_balanced: bool,
    /// Posting date override from a `date:` tag (hledger: used by reports).
    pub date: Option<NaiveDate>,
    /// Secondary posting date from a `date2:` tag.
    pub date2: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PostingAmount {
    pub quantity: Decimal,
    pub commodity: String,
    pub style: AmountStyle,
    pub cost: Option<Cost>,
    /// True for `*N` amounts in auto posting rules: quantity is a multiplier
    /// of the matched posting's amount, and `commodity` is empty.
    pub multiplier: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AmountStyle {
    pub commodity_side: Side,
    pub commodity_spaced: bool,
    pub decimal_mark: char,
    pub precision: u8,
}

impl Default for AmountStyle {
    fn default() -> Self {
        Self {
            commodity_side: Side::Left,
            commodity_spaced: false,
            decimal_mark: '.',
            precision: 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Cost {
    UnitCost(CostAmount),
    TotalCost(CostAmount),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CostAmount {
    pub quantity: Decimal,
    pub commodity: String,
    /// Display style captured from the source, so rewrites preserve precision.
    pub style: AmountStyle,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BalanceAssertion {
    /// `==` (also checks that no other commodities are present).
    pub strong: bool,
    /// `=*` / `==*`: assert against the balance including subaccounts.
    pub inclusive: bool,
    pub quantity: Decimal,
    pub commodity: String,
    /// Display style captured from the source.
    pub style: AmountStyle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountName {
    pub full: String,
    pub parts: Vec<String>,
}

impl AccountName {
    pub fn new(full: &str) -> Self {
        let parts = full.split(':').map(|s| s.to_string()).collect();
        Self {
            full: full.to_string(),
            parts,
        }
    }

    pub fn depth(&self) -> usize {
        self.parts.len()
    }

    /// Returns true if this account is an ancestor of `other`.
    pub fn is_ancestor_of(&self, other: &AccountName) -> bool {
        other.full.starts_with(&self.full) && other.full.len() > self.full.len()
            && other.full.as_bytes().get(self.full.len()) == Some(&b':')
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tag {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Comment {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AccountDirective {
    pub name: AccountName,
    pub comment: Option<Comment>,
    /// Tags from the inline comment and indented subdirective comments,
    /// notably `type:` (A/L/E/R/X/C/V) for statement classification.
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommodityDirective {
    pub commodity: String,
    pub format: Option<AmountStyle>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PriceDirective {
    pub date: NaiveDate,
    pub commodity: String,
    pub price_quantity: Decimal,
    pub price_commodity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IncludeDirective {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AliasDirective {
    pub from: String,
    pub to: String,
    /// True for `alias /regex/ = replacement` form.
    pub regex: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecimalMarkDirective {
    pub mark: char,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PeriodicTransaction {
    /// The full period expression (everything up to the double-space that
    /// separates it from the description), e.g. "every 2 weeks from 2024-01".
    pub period: String,
    pub description: String,
    pub postings: Vec<Posting>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AutoPostingRule {
    pub query: String,
    pub postings: Vec<Posting>,
    pub span: SourceSpan,
}
