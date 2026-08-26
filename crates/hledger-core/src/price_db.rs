use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use hledger_parser::ast::{Cost, Journal, JournalItem};


/// One point in a commodity's price history.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricePoint {
    pub date: String,
    pub rate: String,
}

/// A commodity pair and everything known about its price over time.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceSeries {
    pub base: String,
    pub quote: String,
    pub points: Vec<PricePoint>,
}

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
    /// Uses direct prices, reverse prices, and chained (triangulated)
    /// conversions through intermediate commodities, like hledger -X.
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

        // BFS over the commodity graph (direct + reverse edges), shortest
        // chain wins; depth-capped to keep pathological graphs cheap.
        self.chained_price(from, to, date, 4)
    }

    fn chained_price(
        &self,
        from: &str,
        to: &str,
        date: NaiveDate,
        max_depth: usize,
    ) -> Option<Decimal> {
        use std::collections::{HashMap, HashSet, VecDeque};

        // Build the neighbor map lazily from known pairs with a usable rate.
        let mut edges: HashMap<&str, Vec<(&str, Decimal)>> = HashMap::new();
        for (fc, tc) in self.prices.keys() {
            if let Some(rate) = self.lookup_direct(fc, tc, date) {
                if !rate.is_zero() {
                    edges.entry(fc.as_str()).or_default().push((tc.as_str(), rate));
                    edges
                        .entry(tc.as_str())
                        .or_default()
                        .push((fc.as_str(), Decimal::ONE / rate));
                }
            }
        }

        let mut visited: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<(&str, Decimal, usize)> = VecDeque::new();
        visited.insert(from);
        queue.push_back((from, Decimal::ONE, 0));

        while let Some((commodity, rate, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            if let Some(neighbors) = edges.get(commodity) {
                for (next, edge_rate) in neighbors {
                    if !visited.insert(next) {
                        continue;
                    }
                    let next_rate = rate * edge_rate;
                    if *next == to {
                        return Some(next_rate);
                    }
                    queue.push_back((next, next_rate, depth + 1));
                }
            }
        }
        None
    }

    /// All commodities that have at least one price relationship.
    /// Every priced pair with its full history, oldest first. Reverse pairs
    /// (the inverse rates stored alongside each price) are omitted so a pair
    /// is reported once, in the direction it was written.
    pub fn series(&self) -> Vec<PriceSeries> {
        let mut out: Vec<PriceSeries> = Vec::new();
        for ((from, to), points) in &self.prices {
            // A pair whose inverse is also stored appears twice; keep the one
            // that was declared, which is the one with more points, falling
            // back to a stable ordering when they match.
            if let Some(reverse) = self.prices.get(&(to.clone(), from.clone())) {
                if reverse.len() > points.len() || (reverse.len() == points.len() && to < from) {
                    continue;
                }
            }
            let mut points: Vec<(NaiveDate, Decimal)> = points.clone();
            points.sort_by_key(|(d, _)| *d);
            out.push(PriceSeries {
                base: from.clone(),
                quote: to.clone(),
                points: points
                    .into_iter()
                    .map(|(date, rate)| PricePoint {
                        date: date.format("%Y-%m-%d").to_string(),
                        rate: rate.normalize().to_string(),
                    })
                    .collect(),
            });
        }
        out.sort_by(|a, b| b.points.len().cmp(&a.points.len()).then(a.base.cmp(&b.base)));
        out
    }

    pub fn known_commodities(&self) -> Vec<String> {
        let mut set = std::collections::BTreeSet::new();
        for (f, t) in self.prices.keys() {
            set.insert(f.clone());
            set.insert(t.clone());
        }
        set.into_iter().collect()
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
    fn triangulated_price_lookup() {
        let mut db = PriceDb::new();
        db.add_price(d(2024, 1, 1), "EUR", "USD", dec!(1.10));
        db.add_price(d(2024, 1, 1), "GBP", "USD", dec!(1.25));

        // EUR -> GBP via USD: 1.10 / 1.25 = 0.88
        let rate = db.get_price("EUR", "GBP", d(2024, 1, 15)).unwrap();
        assert_eq!(rate, dec!(0.88));
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
