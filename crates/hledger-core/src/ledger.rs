use std::collections::BTreeSet;

use serde::Serialize;

use hledger_parser::ast::{Journal, Status};

use crate::account::AccountTree;
use crate::balance::{build_account_tree, resolve_journal, ResolvedTransaction, ResolveWarning};
use crate::classify::AccountClassifier;
use crate::styles::CommodityStyles;
use crate::error::LedgerError;
use crate::price_db::PriceDb;

/// A fully resolved ledger with computed balances and account tree.
pub struct Ledger {
    transactions: Vec<ResolvedTransaction>,
    account_tree: AccountTree,
    price_db: PriceDb,
    classifier: AccountClassifier,
    styles: CommodityStyles,
    warnings: Vec<ResolveWarning>,
}

/// A flattened view of a resolved posting for the Tauri command layer.
#[derive(Debug, Clone, Serialize)]
pub struct PostingView {
    pub account: AccountView,
    pub quantity: String,
    pub commodity: String,
}

/// Account info for serialization.
#[derive(Debug, Clone, Serialize)]
pub struct AccountView {
    pub full: String,
    pub parts: Vec<String>,
}

/// A flattened view of a transaction for the Tauri command layer.
pub struct TransactionView<'a> {
    pub date: chrono::NaiveDate,
    pub secondary_date: Option<chrono::NaiveDate>,
    pub status: Status,
    pub code: Option<String>,
    pub description: String,
    pub postings: Vec<PostingViewRef<'a>>,
}

pub struct PostingViewRef<'a> {
    pub account: &'a hledger_parser::ast::AccountName,
    pub amount: &'a crate::amount::MixedAmount,
}

impl Ledger {
    /// Create a Ledger from a parsed Journal.
    pub fn from_journal(journal: &Journal) -> Result<Self, LedgerError> {
        let result = resolve_journal(journal)?;
        let account_tree = build_account_tree(&result.transactions);
        let price_db = PriceDb::from_journal(journal);
        let classifier = AccountClassifier::from_journal(journal);
        let styles = CommodityStyles::from_journal(journal);

        Ok(Self {
            transactions: result.transactions,
            account_tree,
            price_db,
            classifier,
            styles,
            warnings: result.warnings,
        })
    }

    /// Non-fatal problems found during resolution (assertion failures, auto
    /// posting issues). Must be surfaced in the UI.
    pub fn warnings(&self) -> &[ResolveWarning] {
        &self.warnings
    }

    /// The account type classifier (declared types + name inference).
    pub fn classifier(&self) -> &AccountClassifier {
        &self.classifier
    }

    /// Per-commodity display precision, so reports print amounts the way
    /// hledger does rather than however the arithmetic happened to land.
    pub fn styles(&self) -> &CommodityStyles {
        &self.styles
    }

    /// The most-used commodity in the journal, by posting count. Used as the
    /// default valuation target when the user hasn't chosen one — guessing
    /// "$" on a EUR journal produced garbage charts.
    pub fn primary_commodity(&self) -> Option<String> {
        let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
        for txn in &self.transactions {
            for posting in &txn.postings {
                for commodity in posting.amount.amounts.keys() {
                    if !commodity.is_empty() {
                        *counts.entry(commodity).or_insert(0) += 1;
                    }
                }
            }
        }
        counts
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map(|(c, _)| c.to_string())
    }

    /// Number of transactions.
    pub fn transaction_count(&self) -> usize {
        self.transactions.len()
    }

    /// Number of accounts.
    pub fn account_count(&self) -> usize {
        self.account_tree.len()
    }

    /// Iterate over resolved transactions (sorted by date).
    pub fn transactions(&self) -> impl Iterator<Item = &ResolvedTransaction> {
        self.transactions.iter()
    }

    /// Get the account tree.
    pub fn account_tree(&self) -> &AccountTree {
        &self.account_tree
    }

    /// Get the price database.
    pub fn price_db(&self) -> &PriceDb {
        &self.price_db
    }

    /// Get all unique account names, sorted.
    pub fn account_names(&self) -> Vec<String> {
        self.account_tree
            .accounts
            .keys()
            .cloned()
            .collect()
    }

    /// Get account names matching a prefix (case-insensitive).
    /// Accounts previously used with this description, most-used first.
    ///
    /// Entering a transaction on a phone is mostly retyping something you have
    /// entered before; "Whole Foods" almost always means the same two
    /// accounts. Matching is case-insensitive and ignores anything after a
    /// `|`, which hledger treats as the payee/note separator.
    pub fn accounts_for_description(&self, description: &str) -> Vec<String> {
        let key = |s: &str| -> String {
            s.split('|').next().unwrap_or(s).trim().to_lowercase()
        };
        let wanted = key(description);
        if wanted.is_empty() {
            return Vec::new();
        }

        let mut counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for txn in &self.transactions {
            if key(&txn.description) != wanted {
                continue;
            }
            for posting in &txn.postings {
                *counts.entry(posting.account.full.clone()).or_insert(0) += 1;
            }
        }

        let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked.into_iter().map(|(account, _)| account).collect()
    }

    pub fn suggest_accounts(&self, prefix: &str) -> Vec<String> {
        let prefix_lower = prefix.to_lowercase();
        self.account_tree
            .accounts
            .keys()
            .filter(|name| name.to_lowercase().contains(&prefix_lower))
            .cloned()
            .collect()
    }

    /// Get all unique descriptions, sorted by most recent first.
    pub fn descriptions(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        // Iterate in reverse (most recent first since sorted by date)
        for txn in self.transactions.iter().rev() {
            if seen.insert(txn.description.clone()) {
                result.push(txn.description.clone());
            }
        }
        result
    }

    /// Get descriptions matching a prefix (case-insensitive), most recent first.
    pub fn suggest_descriptions(&self, prefix: &str) -> Vec<String> {
        let prefix_lower = prefix.to_lowercase();
        self.descriptions()
            .into_iter()
            .filter(|d| d.to_lowercase().contains(&prefix_lower))
            .collect()
    }

    /// Get all unique payees/descriptions for autocomplete, most recent first.
    pub fn suggest_payees(&self, prefix: &str) -> Vec<String> {
        // In hledger, payee is the description (or part before |)
        self.suggest_descriptions(prefix)
    }

    /// Get the most recently used postings for a given description.
    /// Useful for pre-filling a new transaction based on a previous similar one.
    pub fn last_transaction_for_description(&self, description: &str) -> Option<&ResolvedTransaction> {
        self.transactions
            .iter()
            .rev()
            .find(|t| t.description == description)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_for_description_ranks_by_use() {
        let journal = hledger_parser::parse(concat!(
            "2024-01-05 Whole Foods\n    expenses:food  $50.00\n    assets:checking\n\n",
            "2024-02-05 Whole Foods\n    expenses:food  $60.00\n    assets:checking\n\n",
            "2024-03-05 Whole Foods\n    expenses:food  $20.00\n    assets:cash\n\n",
            "2024-04-05 Petrol\n    expenses:car  $40.00\n    assets:checking\n",
        ))
        .unwrap();
        let ledger = Ledger::from_journal(&journal).unwrap();

        let accounts = ledger.accounts_for_description("whole foods");
        assert_eq!(accounts[0], "expenses:food", "used every time");
        assert_eq!(accounts[1], "assets:checking", "used twice");
        assert!(accounts.contains(&"assets:cash".to_string()));
        assert!(!accounts.contains(&"expenses:car".to_string()), "other payee");

        // The payee/note separator is ignored, and an empty query matches none.
        assert_eq!(ledger.accounts_for_description("Whole Foods | weekly")[0], "expenses:food");
        assert!(ledger.accounts_for_description("  ").is_empty());
    }

    use super::*;
    use hledger_parser::parse;

    #[test]
    fn ledger_from_simple_journal() {
        let journal = parse(
            "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking\n",
        )
        .unwrap();
        let ledger = Ledger::from_journal(&journal).unwrap();

        assert_eq!(ledger.transaction_count(), 1);
        assert!(ledger.account_count() > 0);
    }

    #[test]
    fn ledger_from_empty_journal() {
        let journal = parse("").unwrap();
        let ledger = Ledger::from_journal(&journal).unwrap();
        assert_eq!(ledger.transaction_count(), 0);
        assert_eq!(ledger.account_count(), 0);
    }

    #[test]
    fn ledger_transactions_are_sorted() {
        let journal = parse(
            "2024-01-20 B\n    a  $1\n    b\n\n2024-01-10 A\n    a  $1\n    b\n",
        )
        .unwrap();
        let ledger = Ledger::from_journal(&journal).unwrap();

        let descs: Vec<&str> = ledger.transactions().map(|t| t.description.as_str()).collect();
        assert_eq!(descs, vec!["A", "B"]);
    }

    #[test]
    fn suggest_accounts_all() {
        let journal = parse(
            "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking\n",
        )
        .unwrap();
        let ledger = Ledger::from_journal(&journal).unwrap();

        let accounts = ledger.account_names();
        assert!(accounts.contains(&"expenses:food".to_string()));
        assert!(accounts.contains(&"assets:checking".to_string()));
        assert!(accounts.contains(&"expenses".to_string()));
        assert!(accounts.contains(&"assets".to_string()));
    }

    #[test]
    fn suggest_accounts_filtered() {
        let journal = parse(
            "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking\n\
             2024-01-16 Test2\n    expenses:rent  $100\n    assets:savings\n",
        )
        .unwrap();
        let ledger = Ledger::from_journal(&journal).unwrap();

        let suggestions = ledger.suggest_accounts("exp");
        assert!(suggestions.iter().any(|s| s == "expenses:food"));
        assert!(suggestions.iter().any(|s| s == "expenses:rent"));
        assert!(!suggestions.iter().any(|s| s == "assets:checking"));
    }

    #[test]
    fn suggest_descriptions_most_recent_first() {
        let journal = parse(
            "2024-01-10 Alpha\n    a  $1\n    b\n\n\
             2024-01-20 Beta\n    a  $1\n    b\n\n\
             2024-01-30 Alpha\n    a  $1\n    b\n",
        )
        .unwrap();
        let ledger = Ledger::from_journal(&journal).unwrap();

        let descs = ledger.descriptions();
        // Alpha appears last chronologically (most recent), so should be first
        assert_eq!(descs[0], "Alpha");
        assert_eq!(descs[1], "Beta");
        // No duplicates
        assert_eq!(descs.len(), 2);
    }

    #[test]
    fn suggest_descriptions_filtered() {
        let journal = parse(
            "2024-01-10 Grocery Store\n    a  $1\n    b\n\n\
             2024-01-20 Gas Station\n    a  $1\n    b\n",
        )
        .unwrap();
        let ledger = Ledger::from_journal(&journal).unwrap();

        let suggestions = ledger.suggest_descriptions("gro");
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0], "Grocery Store");
    }

    #[test]
    fn last_transaction_for_description() {
        let journal = parse(
            "2024-01-10 Grocery\n    expenses:food  $30\n    assets:checking\n\n\
             2024-01-20 Grocery\n    expenses:food  $50\n    assets:checking\n",
        )
        .unwrap();
        let ledger = Ledger::from_journal(&journal).unwrap();

        let last = ledger.last_transaction_for_description("Grocery").unwrap();
        // Should be the most recent one (sorted by date)
        assert_eq!(
            last.date,
            chrono::NaiveDate::from_ymd_opt(2024, 1, 20).unwrap()
        );
    }
}
