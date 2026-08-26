/** When on, every formatted amount is masked.
 *
 *  A module-level flag rather than a prop: amounts are formatted from dozens
 *  of call sites, and a display toggle that has to be threaded through all of
 *  them will be missed somewhere — which for this feature means leaking the
 *  number it was meant to hide. */
let incognito = false;

export function setIncognito(on: boolean): void {
  incognito = on;
}

export function isIncognito(): boolean {
  return incognito;
}

/** Masked stand-in. Fixed width regardless of the value, since a mask that
 *  preserved the digit count would still give the magnitude away. */
const MASK = "\u2022\u2022\u2022";

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
  if (incognito) return MASK;
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
  // Keep the commodity: which currency an account holds isn't the secret.
  if (!commodity) return qs;
  return commodity.length === 1 && SYMBOLS.includes(commodity)
    ? `${commodity}${qs}`
    : `${qs} ${commodity}`;
}
