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
const MASK = "•••";

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

/** The locale's grouping and decimal separators, read once from Intl so the
 *  string formatter below never has to round-trip through a float. */
let separators: { group: string; decimal: string } | null = null;

function localeSeparators(): { group: string; decimal: string } {
  if (separators) return separators;
  let group = ",";
  let decimal = ".";
  try {
    for (const part of new Intl.NumberFormat(undefined).formatToParts(1234567.89)) {
      if (part.type === "group") group = part.value;
      else if (part.type === "decimal") decimal = part.value;
    }
  } catch {
    // Keep the defaults.
  }
  separators = { group, decimal };
  return separators;
}

/** Test hook: forget the cached separators so a different locale can be
 *  simulated. */
export function resetSeparatorsForTests(): void {
  separators = null;
}

/** Format a quantity with thousands separators, keeping exactly the digits
 *  the backend chose. Works on the string directly: a float would lose
 *  precision past 15 significant digits, which crypto quantities exceed. */
export function formatQuantity(quantity: string): string {
  if (incognito) return MASK;
  const m = /^\s*([+-]?)(\d+)(?:\.(\d*))?\s*$/.exec(quantity);
  if (!m) return quantity;
  const { group, decimal } = localeSeparators();
  const sign = m[1] === "-" ? "-" : "";
  const int = m[2].replace(/\B(?=(\d{3})+(?!\d))/g, group);
  const frac = m[3] ?? "";
  return frac.length > 0 || quantity.includes(".")
    ? `${sign}${int}${decimal}${frac}`
    : `${sign}${int}`;
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

/** Sign of a dot-decimal string without parsing it as a float: -1, 0 or 1. */
export function decimalSign(quantity: string): -1 | 0 | 1 {
  const s = quantity.trim();
  if (/^[+-]?0*(\.0*)?$/.test(s)) return 0;
  return s.startsWith("-") ? -1 : 1;
}

/** Tailwind classes for an amount's colour: red when negative, green when
 *  positive, neutral for zero. Shared so every screen agrees. */
export function amountTone(quantity: string | null | undefined): string {
  if (!quantity) return "text-gray-400 dark:text-gray-500";
  const sign = decimalSign(quantity);
  if (sign < 0) return "text-negative";
  if (sign > 0) return "text-positive";
  return "text-gray-500 dark:text-gray-400";
}
