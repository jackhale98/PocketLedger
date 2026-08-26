/** Decimal places present in a quantity string as the backend sent it.
 *
 *  The backend already rounds and pads each amount to its commodity's display
 *  precision, the way hledger does — so a report reads "1200.00" for a
 *  two-decimal currency and "3" for a whole-number one. Re-deciding the
 *  precision here would undo that and put the app out of step with the CLI. */
export function decimalsIn(quantity: string): number {
  const dot = quantity.indexOf(".");
  return dot === -1 ? 0 : quantity.length - dot - 1;
}

/** Format a quantity with thousands separators, keeping exactly the decimals
 *  the backend chose. */
export function formatQuantity(quantity: string): string {
  const n = parseFloat(quantity);
  if (Number.isNaN(n)) return quantity;
  const dp = decimalsIn(quantity);
  return n.toLocaleString(undefined, {
    minimumFractionDigits: dp,
    maximumFractionDigits: dp,
  });
}

const SYMBOLS = "$€£¥₹₽₿";

/** Render an amount the way the journal writes it: symbol commodities hug the
 *  number, codes follow it. */
export function formatAmount(quantity: string, commodity: string): string {
  const qs = formatQuantity(quantity);
  if (!commodity) return qs;
  return commodity.length === 1 && SYMBOLS.includes(commodity)
    ? `${commodity}${qs}`
    : `${qs} ${commodity}`;
}
