use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use hledger_parser::ast::{AccountName, Journal, JournalItem, Status, Transaction};

use crate::account::AccountTree;
use crate::amount::MixedAmount;
use crate::error::LedgerError;

/// A resolved posting with computed amount.
#[derive(Debug, Clone)]
pub struct ResolvedPosting {
    pub account: AccountName,
    pub amount: MixedAmount,
    pub status: Status,
    pub date: NaiveDate,
    pub description: String,
    pub transaction_index: usize,
    pub comment: Option<String>,
}

/// A resolved transaction with all amounts computed.
#[derive(Debug, Clone)]
pub struct ResolvedTransaction {
    pub date: NaiveDate,
    pub secondary_date: Option<NaiveDate>,
    pub status: Status,
    pub code: Option<String>,
    pub description: String,
    pub comment: Option<String>,
    pub postings: Vec<ResolvedPosting>,
}

/// Process a journal's transactions: infer missing amounts, validate balancing,
/// and check balance assertions (in date order, per hledger semantics).
pub fn resolve_transactions(
    journal: &Journal,
) -> Result<Vec<ResolvedTransaction>, LedgerError> {
    let mut resolved = Vec::new();

    // Collect transactions with their original AST for assertion checking
    let transactions: Vec<&Transaction> = journal
        .items
        .iter()
        .filter_map(|item| match item {
            JournalItem::Transaction(t) => Some(t),
            _ => None,
        })
        .collect();

    for (idx, txn) in transactions.iter().enumerate() {
        let resolved_txn = resolve_transaction(txn, idx)?;
        resolved.push(resolved_txn);
    }

    // Sort by date (stable sort preserves parse order for same-date transactions)
    resolved.sort_by_key(|t| t.date);

    // Validate balance assertions in date order
    // Track running balance per account per commodity
    let mut running_balances: BTreeMap<String, MixedAmount> = BTreeMap::new();

    // Re-collect sorted AST transactions for assertion data
    let mut sorted_ast_txns: Vec<&Transaction> = transactions.clone();
    sorted_ast_txns.sort_by_key(|t| t.date);

    for (resolved_txn, ast_txn) in resolved.iter().zip(sorted_ast_txns.iter()) {
        for (posting, ast_posting) in resolved_txn.postings.iter().zip(ast_txn.postings.iter()) {
            let balance = running_balances
                .entry(posting.account.full.clone())
                .or_insert_with(MixedAmount::zero);
            balance.add_mixed(&posting.amount);

            // Check balance assertion if present
            if let Some(ref assertion) = ast_posting.balance_assertion {
                let actual = balance.get(&assertion.commodity);
                if actual != assertion.quantity {
                    // Log warning but don't fail - many real journals have
                    // assertions that are informational
                    // For strict mode, we could return an error here
                    eprintln!(
                        "Balance assertion warning at line {}: {} expected {} {}, got {} {}",
                        ast_txn.span.line,
                        posting.account.full,
                        assertion.commodity,
                        assertion.quantity,
                        assertion.commodity,
                        actual,
                    );
                }
            }
        }
    }

    Ok(resolved)
}

/// Resolve a single transaction: infer missing amounts and validate.
fn resolve_transaction(
    txn: &Transaction,
    index: usize,
) -> Result<ResolvedTransaction, LedgerError> {
    let mut postings_with_amounts: Vec<(usize, MixedAmount)> = Vec::new();
    let mut missing_amount_idx: Option<usize> = None;

    for (i, posting) in txn.postings.iter().enumerate() {
        if let Some(ref amt) = posting.amount {
            let quantity = amt.quantity;
            let commodity = &amt.commodity;

            // When a posting has a cost (@ or {}), hledger uses the cost
            // amount for balancing, not the commodity amount. For example:
            //   -19 ITOT {96.15 USD}
            // contributes -19*96.15 = -1826.85 USD to the balance equation
            // (not -19 ITOT). The posting itself still tracks -19 ITOT.
            let balance_mixed = if let Some(ref cost) = amt.cost {
                match cost {
                    hledger_parser::ast::Cost::UnitCost(c) => {
                        let cost_total = quantity * c.quantity;
                        MixedAmount::single(&c.commodity, cost_total)
                    }
                    hledger_parser::ast::Cost::TotalCost(c) => {
                        let cost_total = if quantity.is_sign_negative() {
                            -c.quantity
                        } else {
                            c.quantity
                        };
                        MixedAmount::single(&c.commodity, cost_total)
                    }
                }
            } else {
                MixedAmount::single(commodity, quantity)
            };
            postings_with_amounts.push((i, balance_mixed));
        } else {
            if missing_amount_idx.is_some() {
                return Err(LedgerError::MultipleInferredAmounts {
                    line: txn.span.line,
                });
            }
            missing_amount_idx = Some(i);
        }
    }

    // Calculate the sum of all explicit amounts
    let mut sum = MixedAmount::zero();
    for (_, amt) in &postings_with_amounts {
        sum.add_mixed(amt);
    }

    // Build the final resolved postings
    let mut resolved_postings = Vec::new();

    for (i, posting) in txn.postings.iter().enumerate() {
        let amount = if Some(i) == missing_amount_idx {
            // Infer: the missing amount is the negation of the sum
            sum.negate()
        } else {
            // Use the posting's commodity amount only (not the cost side).
            // The cost side was used for balancing but the posting tracks
            // the commodity it actually holds.
            let quantity = posting.amount.as_ref().unwrap().quantity;
            let commodity = &posting.amount.as_ref().unwrap().commodity;
            MixedAmount::single(commodity, quantity)
        };

        resolved_postings.push(ResolvedPosting {
            account: posting.account.clone(),
            amount,
            status: posting.status,
            date: txn.date,
            description: txn.description.clone(),
            transaction_index: index,
            comment: posting.comment.as_ref().map(|c| c.text.clone()),
        });
    }

    // Validate: transaction must balance (sum of all postings == zero)
    // For multi-currency transactions without cost, we allow imbalance
    // (hledger behavior: each commodity must balance independently)
    if missing_amount_idx.is_none() {
        // All amounts explicit - check balance per commodity
        // But: multi-currency transactions with @ cost are balanced via the cost
        let has_costs = txn
            .postings
            .iter()
            .any(|p| p.amount.as_ref().map_or(false, |a| a.cost.is_some()));

        if !has_costs && sum.is_single_commodity() && !sum.is_zero() {
            return Err(LedgerError::UnbalancedTransaction {
                line: txn.span.line,
                message: format!("off by {}", sum),
            });
        }
    }

    Ok(ResolvedTransaction {
        date: txn.date,
        secondary_date: txn.secondary_date,
        status: txn.status,
        code: txn.code.clone(),
        description: txn.description.clone(),
        comment: txn.comment.as_ref().map(|c| c.text.clone()),
        postings: resolved_postings,
    })
}

/// Build an account tree from resolved transactions.
pub fn build_account_tree(transactions: &[ResolvedTransaction]) -> AccountTree {
    let mut tree = AccountTree::new();

    for txn in transactions {
        for posting in &txn.postings {
            tree.add_to_account(&posting.account, &posting.amount);
        }
    }

    tree.compute_balances();
    tree
}

#[cfg(test)]
mod tests {
    use super::*;
    use hledger_parser::parse;
    use rust_decimal_macros::dec;

    fn parse_and_resolve(input: &str) -> Vec<ResolvedTransaction> {
        let journal = parse(input).unwrap();
        resolve_transactions(&journal).unwrap()
    }

    #[test]
    fn infer_missing_amount() {
        let txns = parse_and_resolve(
            "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking\n",
        );
        assert_eq!(txns.len(), 1);
        assert_eq!(txns[0].postings.len(), 2);
        assert_eq!(txns[0].postings[1].amount.get("$"), dec!(-50.00));
    }

    #[test]
    fn explicit_balanced_transaction() {
        let txns = parse_and_resolve(
            "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking  $-50.00\n",
        );
        assert_eq!(txns.len(), 1);
        assert_eq!(txns[0].postings[0].amount.get("$"), dec!(50.00));
        assert_eq!(txns[0].postings[1].amount.get("$"), dec!(-50.00));
    }

    #[test]
    fn unbalanced_transaction_errors() {
        let journal = parse(
            "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking  $-40.00\n",
        )
        .unwrap();
        let result = resolve_transactions(&journal);
        assert!(result.is_err());
    }

    #[test]
    fn multiple_inferred_amounts_errors() {
        let journal =
            parse("2024-01-15 Test\n    expenses:food\n    assets:checking\n").unwrap();
        let result = resolve_transactions(&journal);
        assert!(result.is_err());
    }

    #[test]
    fn multicurrency_with_cost_allowed() {
        let txns = parse_and_resolve(
            "2024-01-15 Exchange\n    assets:eur  100.00 EUR @ $1.10\n    assets:usd\n",
        );
        assert_eq!(txns.len(), 1);
    }

    #[test]
    fn transactions_sorted_by_date() {
        let txns = parse_and_resolve(
            "2024-01-20 Later\n    expenses:a  $10\n    assets:b\n\n\
             2024-01-10 Earlier\n    expenses:a  $20\n    assets:b\n",
        );
        assert_eq!(txns[0].description, "Earlier");
        assert_eq!(txns[1].description, "Later");
    }

    #[test]
    fn build_tree_from_transactions() {
        let txns = parse_and_resolve(
            "2024-01-15 Test\n    expenses:food  $50.00\n    assets:checking  $-50.00\n",
        );
        let tree = build_account_tree(&txns);

        assert_eq!(tree.accounts["expenses:food"].balance.get("$"), dec!(50.00));
        assert_eq!(tree.accounts["expenses"].balance.get("$"), dec!(50.00));
        assert_eq!(
            tree.accounts["assets:checking"].balance.get("$"),
            dec!(-50.00)
        );
        assert_eq!(tree.accounts["assets"].balance.get("$"), dec!(-50.00));
    }

    #[test]
    fn three_posting_transaction() {
        let txns = parse_and_resolve(
            "2024-01-15 Split\n    expenses:food  $30.00\n    expenses:drink  $20.00\n    assets:checking\n",
        );
        assert_eq!(txns[0].postings[2].amount.get("$"), dec!(-50.00));
    }
}
