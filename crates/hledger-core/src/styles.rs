//! Per-commodity display precision, following hledger.
//!
//! hledger decides how many decimal places a commodity shows from a
//! `commodity` directive if one exists, and otherwise from the most precise
//! amount written for it in the journal. Displayed values are then rounded to
//! that precision. Without this, a report shows `1200` where the journal says
//! `$1200.00`, and a valued amount shows every digit of a non-terminating
//! conversion.
//!
//! Arithmetic stays exact — this governs presentation only, and is applied
//! when an amount is turned into a string.

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use hledger_parser::ast::{Journal, JournalItem};

#[derive(Debug, Clone, Default)]
pub struct CommodityStyles {
    precision: BTreeMap<String, u32>,
}

impl CommodityStyles {
    pub fn from_journal(journal: &Journal) -> Self {
        let mut inferred: BTreeMap<String, u32> = BTreeMap::new();
        let mut declared: BTreeMap<String, u32> = BTreeMap::new();

        for item in &journal.items {
            match item {
                JournalItem::CommodityDirective(cd) => {
                    if let Some(style) = &cd.format {
                        declared.insert(cd.commodity.clone(), style.precision as u32);
                    }
                }
                JournalItem::Transaction(txn) => {
                    for posting in &txn.postings {
                        if let Some(amount) = &posting.amount {
                            let entry = inferred.entry(amount.commodity.clone()).or_insert(0);
                            *entry = (*entry).max(amount.style.precision as u32);
                        }
                    }
                }
                _ => {}
            }
        }

        // A declaration wins over what happens to appear in the file.
        for (commodity, precision) in declared {
            inferred.insert(commodity, precision);
        }
        Self { precision: inferred }
    }

    /// Decimal places to show for a commodity. Unknown commodities fall back
    /// to two, the common case for money.
    pub fn precision(&self, commodity: &str) -> u32 {
        self.precision.get(commodity).copied().unwrap_or(2)
    }

    /// Format a quantity for display: rounded to the commodity's precision and
    /// padded to it, so a two-decimal currency reads "1200.00" not "1200".
    pub fn format(&self, quantity: Decimal, commodity: &str) -> String {
        let dp = self.precision(commodity);
        format!("{:.*}", dp as usize, quantity.round_dp(dp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hledger_parser::parse;
    use rust_decimal_macros::dec;

    #[test]
    fn precision_is_the_most_precise_amount_written() {
        // Verified against hledger 1.50.3: 10.5 + 1.234 FOO reports 11.734,
        // and a whole-number commodity reports without decimals.
        let journal = parse(concat!(
            "2024-01-01 a\n    assets:x   10.5 FOO\n    equity:o\n\n",
            "2024-01-02 b\n    assets:x   1.234 FOO\n    equity:o\n\n",
            "2024-01-03 c\n    assets:y   3 BAR\n    equity:o\n",
        ))
        .unwrap();
        let styles = CommodityStyles::from_journal(&journal);

        assert_eq!(styles.precision("FOO"), 3);
        assert_eq!(styles.precision("BAR"), 0);
        assert_eq!(styles.format(dec!(11.734), "FOO"), "11.734");
        assert_eq!(styles.format(dec!(3), "BAR"), "3");
    }

    #[test]
    fn a_commodity_directive_overrides_what_the_file_happens_to_contain() {
        // hledger reports 11.73, rounding the 3-decimal amount to the declared 2.
        let journal = parse(concat!(
            "commodity 1,000.00 FOO\n\n",
            "2024-01-01 a\n    assets:x   10.5 FOO\n    equity:o\n\n",
            "2024-01-02 b\n    assets:x   1.234 FOO\n    equity:o\n",
        ))
        .unwrap();
        let styles = CommodityStyles::from_journal(&journal);

        assert_eq!(styles.precision("FOO"), 2);
        assert_eq!(styles.format(dec!(11.734), "FOO"), "11.73");
    }

    #[test]
    fn display_precision_pads_as_well_as_rounds() {
        let journal = parse("2024-01-01 a\n    assets:x  $10.00\n    equity:o\n").unwrap();
        let styles = CommodityStyles::from_journal(&journal);
        // A whole amount in a 2dp currency still reads with its decimals.
        assert_eq!(styles.format(dec!(1200), "$"), "1200.00");
        // And a non-terminating conversion is cut to the commodity's places.
        assert_eq!(styles.format(dec!(170.526315789), "$"), "170.53");
    }
}

/// Restyling finished reports.
///
/// Reports do their arithmetic in exact decimals and only then get formatted,
/// so precision is applied in one pass here rather than threaded through every
/// report signature. Anything that reaches the UI as an `AmountEntry` should
/// go through this.
pub mod apply {
    use super::CommodityStyles;
    use crate::reports::{AmountEntry, BalanceRow, FinancialStatement, RegisterRow};
    use rust_decimal::Decimal;

    fn entries(entries: &mut [AmountEntry], styles: &CommodityStyles) {
        for entry in entries {
            if let Ok(q) = entry.quantity.parse::<Decimal>() {
                entry.quantity = styles.format(q, &entry.commodity);
            }
        }
    }

    pub fn balance_rows(rows: &mut [BalanceRow], styles: &CommodityStyles) {
        for row in rows {
            entries(&mut row.amounts, styles);
        }
    }

    pub fn statement(statement: &mut FinancialStatement, styles: &CommodityStyles) {
        for section in &mut statement.sections {
            balance_rows(&mut section.rows, styles);
            entries(&mut section.total, styles);
        }
        entries(&mut statement.net, styles);
    }

    pub fn register_rows(rows: &mut [RegisterRow], styles: &CommodityStyles) {
        for row in rows {
            entries(&mut row.amount, styles);
            entries(&mut row.running_total, styles);
        }
    }

    pub fn periodic(
        report: &mut crate::periodic_report::PeriodicBalanceReport,
        styles: &CommodityStyles,
    ) {
        for row in &mut report.rows {
            for cell in &mut row.amounts {
                entries(cell, styles);
            }
            entries(&mut row.total, styles);
        }
        for cell in &mut report.totals {
            entries(cell, styles);
        }
    }
}
