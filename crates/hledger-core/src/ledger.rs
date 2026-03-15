use std::collections::BTreeSet;

use serde::Serialize;

use hledger_parser::ast::{Journal, JournalItem, Status};

use crate::account::AccountTree;
use crate::balance::{build_account_tree, resolve_transactions, ResolvedTransaction};
use crate::error::LedgerError;
use crate::price_db::PriceDb;

/// A fully resolved ledger with computed balances and account tree.
pub struct Ledger {
    transactions: Vec<ResolvedTransaction>,
    account_tree: AccountTree,
    price_db: PriceDb,
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
        let transactions = resolve_transactions(journal)?;
        let account_tree = build_account_tree(&transactions);
        let price_db = PriceDb::from_journal(journal);

        Ok(Self {
            transactions,
            account_tree,
            price_db,
        })
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
