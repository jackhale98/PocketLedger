use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use hledger_parser::ast::{Cost, Journal, JournalItem, PriceDirective};

use crate::balance::ResolvedTransaction;

/// Price database for currency/commodity conversions.
/// Stores historical prices and supports lookups by date.
#[derive(Debug, Clone)]
pub struct PriceDb {
    /// (from_commodity, to_commodity) -> sorted vec of (date, rate)
    prices: BTreeMap<(String, String), Vec<(NaiveDate, Decimal)>>,
}

impl PriceDb {
    pub fn new() -> Self {
        Self {
            prices: BTreeMap::new(),
        }
    }

    /// Build a PriceDb from a parsed journal's P directives and transaction costs.
    pub fn from_journal(journal: &Journal) -> Self {
        let mut db = Self::new();

        for item in &journal.items {
            match item {
                JournalItem::PriceDirective(pd) => {
                    db.add_price(pd.date, &pd.commodity, &pd.price_commodity, pd.price_quantity);
                }
                JournalItem::Transaction(txn) => {
                    for posting in &txn.postings {
                        if let Some(ref amt) = posting.amount {
                            if let Some(ref cost) = amt.cost {
                                match cost {
                                    Cost::UnitCost(c) => {
                                        db.add_price(
                                            txn.date,
                                            &amt.commodity,
                                            &c.commodity,
                                            c.quantity,
                                        );
                                    }
                                    Cost::TotalCost(c) => {
                                        if !amt.quantity.is_zero() {
                                            let rate = c.quantity / amt.quantity;
                                            db.add_price(
                                                txn.date,
                                                &amt.commodity,
                                                &c.commodity,
                                                rate.abs(),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        db
    }

    /// Add a price entry.
    pub fn add_price(&mut self, date: NaiveDate, from: &str, to: &str, rate: Decimal) {
        let key = (from.to_string(), to.to_string());
        let entries = self.prices.entry(key).or_insert_with(Vec::new);

        // Insert maintaining sorted order by date
        match entries.binary_search_by_key(&date, |(d, _)| *d) {
            Ok(pos) => entries[pos] = (date, rate), // Update existing
            Err(pos) => entries.insert(pos, (date, rate)),
        }
    }

    /// Get the price of `from` in terms of `to` on or before `date`.
    /// Returns the most recent available price.
    pub fn get_price(&self, from: &str, to: &str, date: NaiveDate) -> Option<Decimal> {
        // Direct lookup
        if let Some(rate) = self.lookup_direct(from, to, date) {
            return Some(rate);
        }

        // Reverse lookup (if we know EUR->USD, we can derive USD->EUR)
        if let Some(rate) = self.lookup_direct(to, from, date) {
            if !rate.is_zero() {
                return Some(Decimal::ONE / rate);
            }
        }

        None
    }

    /// Convert a quantity from one commodity to another using the price on or before `date`.
    pub fn convert(
        &self,
        quantity: Decimal,
        from: &str,
        to: &str,
        date: NaiveDate,
    ) -> Option<Decimal> {
        if from == to {
            return Some(quantity);
        }
        let rate = self.get_price(from, to, date)?;
        Some(quantity * rate)
    }

    /// Get the number of price entries.
    pub fn len(&self) -> usize {
        self.prices.values().map(|v| v.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.prices.is_empty()
    }

    fn lookup_direct(&self, from: &str, to: &str, date: NaiveDate) -> Option<Decimal> {
        let key = (from.to_string(), to.to_string());
        let entries = self.prices.get(&key)?;

        // Binary search for the most recent price on or before `date`
        match entries.binary_search_by_key(&date, |(d, _)| *d) {
            Ok(pos) => Some(entries[pos].1),
            Err(0) => None, // All prices are after the requested date
            Err(pos) => Some(entries[pos - 1].1),
        }
    }
}

impl Default for PriceDb {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn direct_price_lookup() {
        let mut db = PriceDb::new();
        db.add_price(d(2024, 1, 1), "EUR", "USD", dec!(1.10));
        db.add_price(d(2024, 2, 1), "EUR", "USD", dec!(1.08));

        assert_eq!(db.get_price("EUR", "USD", d(2024, 1, 15)), Some(dec!(1.10)));
        assert_eq!(db.get_price("EUR", "USD", d(2024, 2, 15)), Some(dec!(1.08)));
        assert_eq!(db.get_price("EUR", "USD", d(2023, 12, 1)), None);
    }

    #[test]
    fn reverse_price_lookup() {
        let mut db = PriceDb::new();
        db.add_price(d(2024, 1, 1), "EUR", "USD", dec!(1.10));

        let rate = db.get_price("USD", "EUR", d(2024, 1, 15)).unwrap();
        // 1 / 1.10 ≈ 0.909...
        assert!(rate > dec!(0.90) && rate < dec!(0.92));
    }

    #[test]
    fn convert_amount() {
        let mut db = PriceDb::new();
        db.add_price(d(2024, 1, 1), "EUR", "USD", dec!(1.10));

        let result = db.convert(dec!(100), "EUR", "USD", d(2024, 1, 15));
        assert_eq!(result, Some(dec!(110.0)));
    }

    #[test]
    fn same_commodity_conversion() {
        let db = PriceDb::new();
        assert_eq!(db.convert(dec!(100), "USD", "USD", d(2024, 1, 1)), Some(dec!(100)));
    }

    #[test]
    fn from_journal_p_directives() {
        let journal = hledger_parser::parse(
            "P 2024-01-01 AAPL $150.00\nP 2024-02-01 AAPL $160.00\n",
        )
        .unwrap();
        let db = PriceDb::from_journal(&journal);

        assert_eq!(db.get_price("AAPL", "$", d(2024, 1, 15)), Some(dec!(150.00)));
        assert_eq!(db.get_price("AAPL", "$", d(2024, 2, 15)), Some(dec!(160.00)));
    }

    #[test]
    fn from_journal_cost_notation() {
        let journal = hledger_parser::parse(
            "2024-01-15 Exchange\n    assets:eur  100.00 EUR @ $1.10\n    assets:usd\n",
        )
        .unwrap();
        let db = PriceDb::from_journal(&journal);

        assert_eq!(db.get_price("EUR", "$", d(2024, 1, 15)), Some(dec!(1.10)));
    }
}
