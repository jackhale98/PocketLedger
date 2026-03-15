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

/// Top-level container: everything in one journal file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Journal {
    pub items: Vec<JournalItem>,
    pub source_path: Option<PathBuf>,
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
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Transaction {
    pub span: SourceSpan,
    pub date: NaiveDate,
    pub secondary_date: Option<NaiveDate>,
    pub status: Status,
    pub code: Option<String>,
    pub description: String,
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
    pub comment: Option<Comment>,
    pub tags: Vec<Tag>,
    pub is_virtual: bool,
    pub virtual_balanced: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PostingAmount {
    pub quantity: Decimal,
    pub commodity: String,
    pub style: AmountStyle,
    pub cost: Option<Cost>,
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
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BalanceAssertion {
    pub strong: bool,
    pub quantity: Decimal,
    pub commodity: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecimalMarkDirective {
    pub mark: char,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PeriodicTransaction {
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
