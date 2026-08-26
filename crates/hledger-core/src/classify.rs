//! Account type classification, following hledger's rules: explicit
//! `account ... ; type:X` declarations win, propagate to subaccounts, and
//! fall back to English account-name inference.

use std::collections::BTreeMap;

use hledger_parser::ast::{Journal, JournalItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountType {
    Asset,
    Liability,
    Equity,
    Revenue,
    Expense,
    Cash,
    Conversion,
    Unknown,
}

impl AccountType {
    fn from_tag(value: &str) -> Option<Self> {
        match value.trim() {
            "A" | "a" => Some(Self::Asset),
            "L" | "l" => Some(Self::Liability),
            "E" | "e" => Some(Self::Equity),
            "R" | "r" => Some(Self::Revenue),
            "X" | "x" => Some(Self::Expense),
            "C" | "c" => Some(Self::Cash),
            "V" | "v" => Some(Self::Conversion),
            _ => None,
        }
    }

    /// Cash accounts are also assets; revenue/expense groupings for statements.
    pub fn is_asset(self) -> bool {
        matches!(self, Self::Asset | Self::Cash)
    }
}

/// Keep only postings to accounts of the given types, dropping transactions
/// left with none.
///
/// Filtering postings rather than rows matters for a periodic report: its
/// column totals are computed from what it was given, so removing rows
/// afterwards would leave totals describing accounts no longer shown.
pub fn retain_postings_of_types(
    transactions: &[crate::balance::ResolvedTransaction],
    classifier: &AccountClassifier,
    types: &[AccountType],
) -> Vec<crate::balance::ResolvedTransaction> {
    transactions
        .iter()
        .filter_map(|txn| {
            let mut kept = txn.clone();
            kept.postings
                .retain(|p| types.contains(&classifier.classify(&p.account.full)));
            (!kept.postings.is_empty()).then_some(kept)
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct AccountClassifier {
    /// Explicitly declared types by account name.
    declared: BTreeMap<String, AccountType>,
}

impl AccountClassifier {
    pub fn from_journal(journal: &Journal) -> Self {
        let mut declared = BTreeMap::new();
        for item in &journal.items {
            if let JournalItem::AccountDirective(ad) = item {
                for tag in &ad.tags {
                    if tag.name == "type" {
                        if let Some(t) = tag.value.as_deref().and_then(AccountType::from_tag) {
                            declared.insert(ad.name.full.clone(), t);
                        }
                    }
                }
            }
        }
        Self { declared }
    }

    /// Classify an account: nearest declared ancestor wins, then name inference
    /// on the top-level component (hledger's fallback regexes).
    pub fn classify(&self, account: &str) -> AccountType {
        // Walk from the account up through its ancestors.
        let mut current = account;
        loop {
            if let Some(t) = self.declared.get(current) {
                return *t;
            }
            match current.rfind(':') {
                Some(pos) => current = &current[..pos],
                None => break,
            }
        }

        // Name-based inference, on the top-level account component.
        let top = account.split(':').next().unwrap_or(account).to_lowercase();
        match top.as_str() {
            "asset" | "assets" | "aktiva" | "cash" => AccountType::Asset,
            "liability" | "liabilities" | "debt" | "debts" | "passiva" => AccountType::Liability,
            "equity" | "capital" => AccountType::Equity,
            "income" | "revenue" | "revenues" | "ertrag" => AccountType::Revenue,
            "expense" | "expenses" | "aufwand" => AccountType::Expense,
            _ => AccountType::Unknown,
        }
    }

    /// True if the account should appear on the cash-flow statement: declared
    /// Cash type, or an asset whose name suggests liquidity (hledger's
    /// fallback: cash|bank|checking|savings).
    pub fn is_cash(&self, account: &str) -> bool {
        // A declared type anywhere in the ancestry decides.
        let mut current = account;
        loop {
            if let Some(t) = self.declared.get(current) {
                return *t == AccountType::Cash;
            }
            match current.rfind(':') {
                Some(pos) => current = &current[..pos],
                None => break,
            }
        }
        if self.classify(account) != AccountType::Asset {
            return false;
        }
        let lower = account.to_lowercase();
        ["cash", "bank", "checking", "savings", "wallet"]
            .iter()
            .any(|kw| lower.contains(kw))
    }

    /// True if there are any explicit declarations (used to decide whether
    /// warnings about unclassifiable accounts are worth emitting).
    pub fn has_declarations(&self) -> bool {
        !self.declared.is_empty()
    }
}

#[cfg(test)]
mod type_filter_tests {
    use super::*;
    use crate::balance::resolve_transactions;
    use hledger_parser::parse;

    #[test]
    fn keeps_only_postings_of_the_wanted_types() {
        let journal = parse(concat!(
            "2024-01-05 Pay\n    assets:checking  $3000\n    income:salary\n\n",
            "2024-01-10 Shop\n    expenses:food  $50\n    assets:checking\n",
        ))
        .unwrap();
        let txns = resolve_transactions(&journal).unwrap();
        let classifier = AccountClassifier::from_journal(&journal);

        let ie = retain_postings_of_types(
            &txns,
            &classifier,
            &[AccountType::Revenue, AccountType::Expense],
        );
        let accounts: Vec<&str> = ie
            .iter()
            .flat_map(|t| t.postings.iter().map(|p| p.account.full.as_str()))
            .collect();
        assert_eq!(accounts, vec!["income:salary", "expenses:food"]);

        // A transaction with nothing of the wanted type drops out entirely.
        let equity = retain_postings_of_types(&txns, &classifier, &[AccountType::Equity]);
        assert!(equity.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hledger_parser::parse;

    #[test]
    fn declared_type_wins_and_propagates() {
        let journal = parse(
            "account aktiva:bank  ; type:A\naccount schulden  ; type:L\n",
        )
        .unwrap();
        let c = AccountClassifier::from_journal(&journal);
        assert_eq!(c.classify("aktiva:bank"), AccountType::Asset);
        assert_eq!(c.classify("aktiva:bank:giro"), AccountType::Asset);
        assert_eq!(c.classify("schulden:kredit"), AccountType::Liability);
    }

    #[test]
    fn name_inference_fallback() {
        let c = AccountClassifier::default();
        assert_eq!(c.classify("assets:checking"), AccountType::Asset);
        assert_eq!(c.classify("liabilities:card"), AccountType::Liability);
        assert_eq!(c.classify("income:salary"), AccountType::Revenue);
        assert_eq!(c.classify("revenue:sales"), AccountType::Revenue);
        assert_eq!(c.classify("expenses:food"), AccountType::Expense);
        assert_eq!(c.classify("equity:opening"), AccountType::Equity);
        assert_eq!(c.classify("weird:thing"), AccountType::Unknown);
    }

    #[test]
    fn subaccount_type_override() {
        let journal = parse(
            "account assets:receivable  ; type:A\naccount assets:pension  ; type:L\n",
        )
        .unwrap();
        let c = AccountClassifier::from_journal(&journal);
        assert_eq!(c.classify("assets:pension"), AccountType::Liability);
        assert_eq!(c.classify("assets:receivable"), AccountType::Asset);
    }

    #[test]
    fn cash_detection() {
        let c = AccountClassifier::default();
        assert!(c.is_cash("assets:bank:checking"));
        assert!(c.is_cash("assets:cash"));
        assert!(!c.is_cash("assets:investments:etrade"));
        assert!(!c.is_cash("expenses:food"));

        let journal = parse("account assets:broker:sweep  ; type:C\n").unwrap();
        let c = AccountClassifier::from_journal(&journal);
        assert!(c.is_cash("assets:broker:sweep"));
    }
}
