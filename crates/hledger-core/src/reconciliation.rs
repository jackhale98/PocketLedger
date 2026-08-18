use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Serialize;

use hledger_parser::ast::Status;

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
    /// The posting's EFFECTIVE status (transaction status flows down to
    /// unmarked postings) — comparing toggles against this makes un-clearing
    /// a `*`-transaction posting a real change, and leaving cleared postings
    /// untouched a non-change.
    pub original_status: Status,
}

fn account_matches(posting_account: &str, account: &str) -> bool {
    let pa = posting_account.to_lowercase();
    let a = account.to_lowercase();
    pa == a || (pa.starts_with(&a) && pa.as_bytes().get(a.len()) == Some(&b':'))
}

impl ReconciliationSession {
    /// Start a new reconciliation session for an account. Only real (non-
    /// virtual) postings in the statement's commodity participate — a bank
    /// statement never contains virtual budget postings or other commodities.
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
            for (pi, posting) in txn.postings.iter().enumerate() {
                if posting.date > statement_date {
                    continue;
                }
                if posting.is_virtual {
                    continue;
                }
                if !account_matches(&posting.account.full, account) {
                    continue;
                }

                // Exactly the statement commodity — no cross-commodity summing.
                let amount = posting.amount.get(commodity);
                if amount.is_zero() && !posting.amount.amounts.contains_key(commodity) {
                    continue;
                }

                let is_cleared = posting.status == Status::Cleared;

                posting_statuses.push((ti, pi, is_cleared));
                posting_data.push(PostingData {
                    transaction_index: ti,
                    posting_index: pi,
                    date: posting.date,
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

    /// Toggle a posting's cleared status.
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

    /// Get the list of status changes to apply to the journal:
    /// (transaction_index, posting_index, new_status) for each posting whose
    /// checkbox now differs from its original effective status.
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

        assert_eq!(session.cleared_balance(), dec!(0));
        assert_eq!(session.difference(), dec!(-1050));
        assert!(!session.is_reconciled());

        session.toggle_posting(0);
        assert_eq!(session.cleared_balance(), dec!(-50));
        assert_eq!(session.difference(), dec!(-1000));

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

        assert!(session.posting_statuses[0].2);
        assert!(!session.posting_statuses[1].2);
        assert_eq!(session.cleared_balance(), dec!(-50));
    }

    #[test]
    fn unclearing_txn_cleared_posting_is_a_change() {
        // The posting inherits `*` from the transaction; unchecking it must
        // produce a change (the audit found unchecks were silently dropped).
        let txns = resolve(
            "2024-01-10 * A\n    expenses:food  $50\n    assets:checking  $-50\n",
        );
        let mut session = ReconciliationSession::new(
            &txns, "assets:checking", d(2024, 1, 31), dec!(-50), "$",
        );
        assert!(session.posting_statuses[0].2, "starts cleared");

        // Untouched: no spurious changes.
        assert!(session.changes().is_empty());

        session.toggle_posting(0);
        let changes = session.changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].2, Status::Unmarked);
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
        assert_eq!(session.posting_data.len(), 1);
    }

    #[test]
    fn virtual_and_foreign_commodity_postings_excluded() {
        let txns = resolve(
            "2024-01-10 A\n    (assets:checking)  $99\n    \n\n\
             2024-01-11 B\n    assets:checking  100 EUR\n    assets:eur  -100 EUR\n\n\
             2024-01-12 C\n    assets:checking  $40\n    income\n",
        );
        let session = ReconciliationSession::new(
            &txns, "assets:checking", d(2024, 1, 31), dec!(40), "$",
        );
        // Only the $40 real posting participates.
        assert_eq!(session.posting_data.len(), 1);
        assert_eq!(session.posting_data[0].amount, dec!(40));
    }

    #[test]
    fn account_prefix_boundary() {
        let txns = resolve(
            "2024-01-10 A\n    assets:bank  $30\n    equity\n\n\
             2024-01-11 B\n    assets:bankloan  $20\n    equity\n",
        );
        let session = ReconciliationSession::new(
            &txns, "assets:bank", d(2024, 1, 31), dec!(30), "$",
        );
        assert_eq!(session.posting_data.len(), 1);
        assert_eq!(session.posting_data[0].amount, dec!(30));
    }
}
