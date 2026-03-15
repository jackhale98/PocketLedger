use std::collections::BTreeMap;

use hledger_parser::ast::AccountName;

use crate::amount::MixedAmount;

/// Account tree node. Each account can have children and a balance.
#[derive(Debug, Clone)]
pub struct AccountTree {
    pub accounts: BTreeMap<String, AccountNode>,
}

#[derive(Debug, Clone)]
pub struct AccountNode {
    pub name: AccountName,
    /// Direct balance (only from postings to this exact account).
    pub own_balance: MixedAmount,
    /// Inclusive balance (own + all descendants).
    pub balance: MixedAmount,
    pub children: Vec<String>,
}

impl AccountTree {
    pub fn new() -> Self {
        Self {
            accounts: BTreeMap::new(),
        }
    }

    /// Ensure an account and all its parents exist in the tree.
    pub fn ensure_account(&mut self, name: &AccountName) {
        // Ensure all ancestor accounts exist
        for depth in 1..=name.parts.len() {
            let ancestor_full = name.parts[..depth].join(":");
            if !self.accounts.contains_key(&ancestor_full) {
                self.accounts.insert(
                    ancestor_full.clone(),
                    AccountNode {
                        name: AccountName::new(&ancestor_full),
                        own_balance: MixedAmount::zero(),
                        balance: MixedAmount::zero(),
                        children: Vec::new(),
                    },
                );

                // Register as child of parent
                if depth > 1 {
                    let parent_full = name.parts[..depth - 1].join(":");
                    if let Some(parent) = self.accounts.get_mut(&parent_full) {
                        if !parent.children.contains(&ancestor_full) {
                            parent.children.push(ancestor_full);
                        }
                    }
                }
            }
        }
    }

    /// Add an amount to an account's own balance.
    pub fn add_to_account(&mut self, name: &AccountName, amount: &MixedAmount) {
        self.ensure_account(name);
        if let Some(node) = self.accounts.get_mut(&name.full) {
            node.own_balance.add_mixed(amount);
        }
    }

    /// Recompute inclusive balances from own balances.
    pub fn compute_balances(&mut self) {
        // Get all account names sorted by depth (deepest first)
        let mut names: Vec<String> = self.accounts.keys().cloned().collect();
        names.sort_by(|a, b| {
            let da = a.matches(':').count();
            let db = b.matches(':').count();
            db.cmp(&da).then(a.cmp(b))
        });

        // Reset all inclusive balances
        for node in self.accounts.values_mut() {
            node.balance = node.own_balance.clone();
        }

        // Propagate from leaves to roots
        for name in &names {
            let balance = self.accounts[name].balance.clone();
            if let Some(last_colon) = name.rfind(':') {
                let parent = &name[..last_colon];
                if let Some(parent_node) = self.accounts.get_mut(parent) {
                    parent_node.balance.add_mixed(&balance);
                }
            }
        }
    }

    /// Get all top-level account names.
    pub fn top_level_accounts(&self) -> Vec<&str> {
        self.accounts
            .keys()
            .filter(|name| !name.contains(':'))
            .map(|s| s.as_str())
            .collect()
    }

    /// Get account count.
    pub fn len(&self) -> usize {
        self.accounts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn ensure_creates_hierarchy() {
        let mut tree = AccountTree::new();
        tree.ensure_account(&AccountName::new("assets:bank:checking"));

        assert!(tree.accounts.contains_key("assets"));
        assert!(tree.accounts.contains_key("assets:bank"));
        assert!(tree.accounts.contains_key("assets:bank:checking"));
    }

    #[test]
    fn add_and_compute_balances() {
        let mut tree = AccountTree::new();
        let checking = AccountName::new("assets:bank:checking");
        let savings = AccountName::new("assets:bank:savings");

        tree.add_to_account(&checking, &MixedAmount::single("$", dec!(1000)));
        tree.add_to_account(&savings, &MixedAmount::single("$", dec!(5000)));
        tree.compute_balances();

        // Own balances
        assert_eq!(tree.accounts["assets:bank:checking"].own_balance.get("$"), dec!(1000));
        assert_eq!(tree.accounts["assets:bank:savings"].own_balance.get("$"), dec!(5000));

        // Inclusive balances
        assert_eq!(tree.accounts["assets:bank"].balance.get("$"), dec!(6000));
        assert_eq!(tree.accounts["assets"].balance.get("$"), dec!(6000));
    }

    #[test]
    fn top_level_accounts() {
        let mut tree = AccountTree::new();
        tree.ensure_account(&AccountName::new("assets:bank:checking"));
        tree.ensure_account(&AccountName::new("expenses:food"));

        let top = tree.top_level_accounts();
        assert!(top.contains(&"assets"));
        assert!(top.contains(&"expenses"));
        assert_eq!(top.len(), 2);
    }
}
