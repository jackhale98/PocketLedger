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
    /// The codes hledger 1.50 accepts in a `type:` tag, case-insensitively:
    /// "A, L, E, R, X, C, V, Asset, Liability, Equity, Revenue, Expense,
    /// Cash, Conversion" (its own error message). Plurals are rejected.
    pub fn from_tag(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "a" | "asset" => Some(Self::Asset),
            "l" | "liability" => Some(Self::Liability),
            "e" | "equity" => Some(Self::Equity),
            "r" | "revenue" => Some(Self::Revenue),
            "x" | "expense" => Some(Self::Expense),
            "c" | "cash" => Some(Self::Cash),
            "v" | "conversion" => Some(Self::Conversion),
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

fn cash_name_regex() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"(?i)^assets?(:.+)?:(cash|bank|che(ck|que)(ing)?|savings?|current)(:|$)")
            .expect("static regex")
    })
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
    /// Cash type, or an asset whose name matches hledger's fallback regex
    /// `^assets?(:.+)?:(cash|bank|che(ck|que)(ing)?|savings?|current)(:|$)`
    /// (verified with `hledger accounts --types` 1.50.3: `assets:wallet` and
    /// `assets:bankruptcy` are plain assets, `assets:investments:cash` and
    /// `asset:bank` are cash).
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
        cash_name_regex().is_match(account)
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
    fn full_word_type_tags_are_accepted_case_insensitively() {
        // `hledger accounts --types` 1.50.3 on these declarations.
        let journal = parse(concat!(
            "account vault  ; type: Asset\n",
            "account loan  ; type: LIABILITY\n",
            "account owner  ; type: equity\n",
            "account sales  ; type: Revenue\n",
            "account rent  ; type:Expense\n",
            "account till  ; type: Cash\n",
            "account fx  ; type: Conversion\n",
            "account plural  ; type: Assets\n",
        ))
        .unwrap();
        let c = AccountClassifier::from_journal(&journal);
        assert_eq!(c.classify("vault"), AccountType::Asset);
        assert_eq!(c.classify("loan"), AccountType::Liability);
        assert_eq!(c.classify("owner"), AccountType::Equity);
        assert_eq!(c.classify("sales"), AccountType::Revenue);
        assert_eq!(c.classify("rent"), AccountType::Expense);
        assert_eq!(c.classify("till"), AccountType::Cash);
        assert_eq!(c.classify("fx"), AccountType::Conversion);
        // hledger rejects "Assets" outright; we ignore the declaration.
        assert_eq!(c.classify("plural"), AccountType::Unknown);
    }

    #[test]
    fn cash_detection() {
        let c = AccountClassifier::default();
        assert!(c.is_cash("assets:bank:checking"));
        assert!(c.is_cash("assets:cash"));
        assert!(!c.is_cash("assets:investments:etrade"));
        assert!(!c.is_cash("expenses:food"));
        // hledger's fallback regex, checked with `accounts --types`.
        assert!(c.is_cash("assets:cheque"));
        assert!(c.is_cash("assets:current"));
        assert!(c.is_cash("assets:saving"));
        assert!(c.is_cash("assets:investments:cash"));
        assert!(c.is_cash("asset:bank"));
        assert!(c.is_cash("Assets:Bank:Foo"));
        assert!(!c.is_cash("assets:wallet"));
        assert!(!c.is_cash("assets:bankruptcy"));
        assert!(!c.is_cash("assets"));

        let journal = parse("account assets:broker:sweep  ; type:C\n").unwrap();
        let c = AccountClassifier::from_journal(&journal);
        assert!(c.is_cash("assets:broker:sweep"));
    }
}
