use rust_decimal::Decimal;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;

/// A multi-commodity amount. Each commodity is tracked independently.
/// This is the fundamental building block for all balance calculations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MixedAmount {
    /// Commodity name -> quantity. Using BTreeMap for deterministic ordering.
    pub amounts: BTreeMap<String, Decimal>,
}

impl MixedAmount {
    pub fn zero() -> Self {
        Self {
            amounts: BTreeMap::new(),
        }
    }

    /// Create a single-commodity amount.
    pub fn single(commodity: &str, quantity: Decimal) -> Self {
        let mut amounts = BTreeMap::new();
        if !quantity.is_zero() {
            amounts.insert(commodity.to_string(), quantity);
        }
        Self { amounts }
    }

    /// Add a quantity of a commodity.
    pub fn add(&mut self, commodity: &str, quantity: Decimal) {
        let entry = self.amounts.entry(commodity.to_string()).or_insert(Decimal::ZERO);
        *entry += quantity;
        // Remove zero entries to keep the map clean
        if entry.is_zero() {
            self.amounts.remove(commodity);
        }
    }

    /// Add another MixedAmount.
    pub fn add_mixed(&mut self, other: &MixedAmount) {
        for (commodity, quantity) in &other.amounts {
            self.add(commodity, *quantity);
        }
    }

    /// Subtract another MixedAmount.
    pub fn subtract(&mut self, other: &MixedAmount) {
        for (commodity, quantity) in &other.amounts {
            self.add(commodity, -*quantity);
        }
    }

    /// Negate all amounts.
    pub fn negate(&self) -> Self {
        let amounts = self
            .amounts
            .iter()
            .map(|(k, v)| (k.clone(), -*v))
            .collect();
        Self { amounts }
    }

    /// True if all quantities are zero (or empty).
    pub fn is_zero(&self) -> bool {
        self.amounts.values().all(|v| v.is_zero())
    }

    /// True if this amount contains exactly one commodity.
    pub fn is_single_commodity(&self) -> bool {
        self.amounts.len() <= 1
    }

    /// Get the quantity for a specific commodity.
    pub fn get(&self, commodity: &str) -> Decimal {
        self.amounts.get(commodity).copied().unwrap_or(Decimal::ZERO)
    }

    /// Number of distinct commodities.
    pub fn commodity_count(&self) -> usize {
        self.amounts.len()
    }
}

impl fmt::Display for MixedAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.amounts.is_empty() {
            return write!(f, "0");
        }
        let parts: Vec<String> = self
            .amounts
            .iter()
            .map(|(commodity, quantity)| {
                if commodity.is_empty() {
                    quantity.to_string()
                } else {
                    format!("{} {}", quantity, commodity)
                }
            })
            .collect();
        write!(f, "{}", parts.join(", "))
    }
}

impl Default for MixedAmount {
    fn default() -> Self {
        Self::zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn zero_amount_is_zero() {
        let amt = MixedAmount::zero();
        assert!(amt.is_zero());
        assert!(amt.is_single_commodity());
    }

    #[test]
    fn single_commodity() {
        let amt = MixedAmount::single("$", dec!(100));
        assert!(!amt.is_zero());
        assert!(amt.is_single_commodity());
        assert_eq!(amt.get("$"), dec!(100));
    }

    #[test]
    fn add_same_commodity() {
        let mut amt = MixedAmount::zero();
        amt.add("$", dec!(100));
        amt.add("$", dec!(50));
        assert_eq!(amt.get("$"), dec!(150));
        assert!(amt.is_single_commodity());
    }

    #[test]
    fn add_different_commodities() {
        let mut amt = MixedAmount::zero();
        amt.add("$", dec!(100));
        amt.add("EUR", dec!(50));
        assert!(!amt.is_single_commodity());
        assert_eq!(amt.get("$"), dec!(100));
        assert_eq!(amt.get("EUR"), dec!(50));
    }

    #[test]
    fn subtract_to_zero() {
        let mut amt = MixedAmount::single("$", dec!(100));
        let other = MixedAmount::single("$", dec!(100));
        amt.subtract(&other);
        assert!(amt.is_zero());
        // Zero entries should be cleaned up
        assert_eq!(amt.amounts.len(), 0);
    }

    #[test]
    fn negate() {
        let amt = MixedAmount::single("$", dec!(100));
        let neg = amt.negate();
        assert_eq!(neg.get("$"), dec!(-100));
    }

    #[test]
    fn add_mixed() {
        let mut a = MixedAmount::single("$", dec!(100));
        let b = MixedAmount::single("EUR", dec!(50));
        a.add_mixed(&b);
        assert_eq!(a.get("$"), dec!(100));
        assert_eq!(a.get("EUR"), dec!(50));
    }

    #[test]
    fn display_format() {
        let mut amt = MixedAmount::zero();
        amt.add("$", dec!(100));
        assert_eq!(format!("{}", amt), "100 $");
    }

    #[test]
    fn display_zero() {
        let amt = MixedAmount::zero();
        assert_eq!(format!("{}", amt), "0");
    }

    #[test]
    fn get_missing_commodity() {
        let amt = MixedAmount::single("$", dec!(100));
        assert_eq!(amt.get("EUR"), dec!(0));
    }
}
