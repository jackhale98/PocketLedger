use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Serialize;

use hledger_parser::ast::Status;

use crate::amount::MixedAmount;
use crate::balance::ResolvedTransaction;

/// A posting reference for the reconciliation UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationPosting {
    /// Index of the transaction in the resolved list.
    pub transaction_index: usize,
    /// Index of the posting within the transaction.
    pub posting_index: usize,
    pub date: String,
    pub description: String,
    pub amount: String,
    pub commodity: String,
    /// Whether this posting is currently marked as cleared.
    pub is_cleared: bool,
}

/// The current state of a reconciliation session.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationState {
    pub account: String,
    pub statement_date: String,
    pub statement_balance: String,
    pub statement_commodity: String,
    pub cleared_balance: String,
    pub difference: String,
    pub is_reconciled: bool,
    pub postings: Vec<ReconciliationPosting>,
}

/// A reconciliation session that tracks which postings are cleared.
pub struct ReconciliationSession {
    pub account: String,
    pub statement_date: NaiveDate,
    pub statement_balance: Decimal,
    pub commodity: String,
    /// (transaction_index, posting_index, is_cleared)
    pub posting_statuses: Vec<(usize, usize, bool)>,
    /// Cached posting data for the UI
    pub posting_data: Vec<PostingData>,
}

pub struct PostingData {
    pub transaction_index: usize,
    pub posting_index: usize,
    pub date: NaiveDate,
    pub description: String,
    pub amount: Decimal,
    pub original_status: Status,
}

impl ReconciliationSession {
    /// Start a new reconciliation session for an account.
    pub fn new(
        transactions: &[ResolvedTransaction],
        account: &str,
        statement_date: NaiveDate,
        statement_balance: Decimal,
        commodity: &str,
    ) -> Self {
        let mut posting_statuses = Vec::new();
        let mut posting_data = Vec::new();

        for (ti, txn) in transactions.iter().enumerate() {
            if txn.date > statement_date {
                continue;
            }
            for (pi, posting) in txn.postings.iter().enumerate() {
                if !posting.account.full.eq_ignore_ascii_case(account)
                    && !posting.account.full.to_lowercase().starts_with(
                        &format!("{}:", account.to_lowercase()),
                    )
                {
                    // Only exact match for reconciliation
                    if posting.account.full.to_lowercase() != account.to_lowercase() {
                        continue;
                    }
                }

                let amount = crate::reports::get_primary_value_pub(&posting.amount, commodity);
                let is_cleared = posting.status == Status::Cleared
                    || txn.status == Status::Cleared;

                posting_statuses.push((ti, pi, is_cleared));
                posting_data.push(PostingData {
                    transaction_index: ti,
                    posting_index: pi,
                    date: txn.date,
                    description: txn.description.clone(),
                    amount,
                    original_status: posting.status,
                });
            }
        }

        Self {
            account: account.to_string(),
            statement_date,
            statement_balance,
            commodity: commodity.to_string(),
            posting_statuses,
            posting_data,
        }
    }

    /// Toggle a posting's cleared status. Returns the posting list index.
    pub fn toggle_posting(&mut self, index: usize) {
        if index < self.posting_statuses.len() {
            self.posting_statuses[index].2 = !self.posting_statuses[index].2;
        }
    }

    /// Calculate the cleared balance (sum of all cleared postings).
    pub fn cleared_balance(&self) -> Decimal {
        self.posting_statuses
            .iter()
            .enumerate()
            .filter(|(_, (_, _, cleared))| *cleared)
            .map(|(i, _)| self.posting_data[i].amount)
            .sum()
    }

    /// The difference between statement balance and cleared balance.
    pub fn difference(&self) -> Decimal {
        self.statement_balance - self.cleared_balance()
    }

    /// Whether the reconciliation is complete (difference is zero).
    pub fn is_reconciled(&self) -> bool {
        self.difference().is_zero()
    }

    /// Get the current state for the UI.
    pub fn state(&self) -> ReconciliationState {
        let cleared = self.cleared_balance();
        let diff = self.difference();

        let postings = self
            .posting_statuses
            .iter()
            .enumerate()
            .map(|(i, (ti, pi, is_cleared))| {
                let data = &self.posting_data[i];
                ReconciliationPosting {
                    transaction_index: *ti,
                    posting_index: *pi,
                    date: data.date.format("%Y-%m-%d").to_string(),
                    description: data.description.clone(),
                    amount: data.amount.to_string(),
                    commodity: self.commodity.clone(),
                    is_cleared: *is_cleared,
                }
            })
            .collect();

        ReconciliationState {
            account: self.account.clone(),
            statement_date: self.statement_date.format("%Y-%m-%d").to_string(),
            statement_balance: self.statement_balance.to_string(),
            statement_commodity: self.commodity.clone(),
            cleared_balance: cleared.to_string(),
            difference: diff.to_string(),
            is_reconciled: diff.is_zero(),
            postings,
        }
    }

    /// Get the list of status changes to apply to the journal.
    /// Returns (transaction_index, posting_index, new_status) for each changed posting.
    pub fn changes(&self) -> Vec<(usize, usize, Status)> {
        self.posting_statuses
            .iter()
            .enumerate()
            .filter_map(|(i, (ti, pi, is_cleared))| {
                let original_cleared = self.posting_data[i].original_status == Status::Cleared;
                if *is_cleared != original_cleared {
                    let new_status = if *is_cleared {
                        Status::Cleared
                    } else {
                        Status::Unmarked
                    };
                    Some((*ti, *pi, new_status))
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::balance::resolve_transactions;
    use hledger_parser::parse;
    use rust_decimal_macros::dec;

    fn resolve(input: &str) -> Vec<ResolvedTransaction> {
        let journal = parse(input).unwrap();
        resolve_transactions(&journal).unwrap()
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn new_session_shows_all_postings() {
        let txns = resolve(
            "2024-01-10 A\n    expenses:food  $50\n    assets:checking  $-50\n\n\
             2024-01-20 B\n    expenses:rent  $1000\n    assets:checking  $-1000\n",
        );
        let session = ReconciliationSession::new(
            &txns, "assets:checking", d(2024, 1, 31), dec!(-1050), "$",
        );
        assert_eq!(session.posting_data.len(), 2);
    }

    #[test]
    fn cleared_balance_tracks_toggled_postings() {
        let txns = resolve(
            "2024-01-10 A\n    expenses:food  $50\n    assets:checking  $-50\n\n\
             2024-01-20 B\n    expenses:rent  $1000\n    assets:checking  $-1000\n",
        );
        let mut session = ReconciliationSession::new(
            &txns, "assets:checking", d(2024, 1, 31), dec!(-1050), "$",
        );

        // Nothing cleared yet
        assert_eq!(session.cleared_balance(), dec!(0));
        assert_eq!(session.difference(), dec!(-1050));
        assert!(!session.is_reconciled());

        // Clear first posting
        session.toggle_posting(0);
        assert_eq!(session.cleared_balance(), dec!(-50));
        assert_eq!(session.difference(), dec!(-1000));

        // Clear second posting
        session.toggle_posting(1);
        assert_eq!(session.cleared_balance(), dec!(-1050));
        assert_eq!(session.difference(), dec!(0));
        assert!(session.is_reconciled());
    }

    #[test]
    fn toggle_twice_returns_to_original() {
        let txns = resolve(
            "2024-01-10 A\n    expenses:food  $50\n    assets:checking  $-50\n",
        );
        let mut session = ReconciliationSession::new(
            &txns, "assets:checking", d(2024, 1, 31), dec!(-50), "$",
        );

        session.toggle_posting(0);
        assert!(session.is_reconciled());

        session.toggle_posting(0);
        assert!(!session.is_reconciled());
    }

    #[test]
    fn already_cleared_postings_start_cleared() {
        let txns = resolve(
            "2024-01-10 * A\n    expenses:food  $50\n    assets:checking  $-50\n\n\
             2024-01-20 B\n    expenses:rent  $1000\n    assets:checking  $-1000\n",
        );
        let session = ReconciliationSession::new(
            &txns, "assets:checking", d(2024, 1, 31), dec!(-1050), "$",
        );

        // First transaction is cleared, so its posting starts cleared
        assert!(session.posting_statuses[0].2);
        // Second is not
        assert!(!session.posting_statuses[1].2);
        assert_eq!(session.cleared_balance(), dec!(-50));
    }

    #[test]
    fn changes_returns_only_modified() {
        let txns = resolve(
            "2024-01-10 A\n    expenses:food  $50\n    assets:checking  $-50\n\n\
             2024-01-20 B\n    expenses:rent  $1000\n    assets:checking  $-1000\n",
        );
        let mut session = ReconciliationSession::new(
            &txns, "assets:checking", d(2024, 1, 31), dec!(-1050), "$",
        );

        // Toggle both to cleared
        session.toggle_posting(0);
        session.toggle_posting(1);

        let changes = session.changes();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].2, Status::Cleared);
    }

    #[test]
    fn date_filter_excludes_future_transactions() {
        let txns = resolve(
            "2024-01-10 A\n    assets:checking  $100\n    income:salary\n\n\
             2024-02-10 B\n    assets:checking  $200\n    income:salary\n",
        );
        let session = ReconciliationSession::new(
            &txns, "assets:checking", d(2024, 1, 31), dec!(100), "$",
        );
        // Only Jan transaction should appear
        assert_eq!(session.posting_data.len(), 1);
    }
}
