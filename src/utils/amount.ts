/**
 * Helpers for parsing user-entered amounts and doing exact decimal math.
 *
 * Normalization rules (see normalizeAmountInput):
 * - Whitespace is trimmed; a leading "+" or "-" sign is allowed.
 * - Spaces, non-breaking spaces, and apostrophes inside the number are
 *   treated as grouping separators and removed ("1 234.56" -> "1234.56").
 * - If both "." and "," are present, the LAST-occurring separator is the
 *   decimal mark and the other is grouping ("1.234,56" -> "1234.56",
 *   "1,234.56" -> "1234.56").
 * - If only "," is present: a single comma followed by 1 or 2 digits is a
 *   decimal mark ("1,5" -> "1.5"); otherwise commas are grouping
 *   ("1,234" -> "1234", "1,234,567" -> "1234567").
 * - If only "." is present it is kept as the decimal mark (more than one
 *   dot is invalid).
 */

/**
 * Normalize a user-entered amount to a dot-decimal string suitable for the
 * backend (e.g. "1,5" -> "1.5"). Returns null if the input is not a
 * recognizable number.
 */
export function normalizeAmountInput(raw: string): string | null {
  let s = raw.trim();
  if (s === "") return null;

  let sign = "";
  if (s.startsWith("+") || s.startsWith("-")) {
    sign = s[0] === "-" ? "-" : "";
    s = s.slice(1);
  }

  // Strip grouping spaces/apostrophes (e.g. "1 234,56" or "1'234.56")
  // \u00a0 no-break, \u202f narrow no-break and \u2009 thin spaces all
  // appear as grouping marks in locale-formatted numbers.
  s = s.replace(/[\s\u00a0\u202f\u2009']/g, "");
  if (s === "" || !/^[\d.,]+$/.test(s)) return null;

  const lastDot = s.lastIndexOf(".");
  const lastComma = s.lastIndexOf(",");
  let normalized: string;

  if (lastDot !== -1 && lastComma !== -1) {
    // Both separators present: the last one is the decimal mark.
    const decIndex = Math.max(lastDot, lastComma);
    const intPart = s.slice(0, decIndex).replace(/[.,]/g, "");
    const fracPart = s.slice(decIndex + 1);
    if (/[.,]/.test(fracPart)) return null;
    normalized = `${intPart}.${fracPart}`;
  } else if (lastComma !== -1) {
    const commaCount = s.split(",").length - 1;
    const frac = s.slice(lastComma + 1);
    if (commaCount === 1 && frac.length >= 1 && frac.length <= 2) {
      // Lone comma with <=2 trailing digits: decimal mark ("1,5" -> "1.5")
      normalized = `${s.slice(0, lastComma)}.${frac}`;
    } else {
      // Otherwise commas are grouping ("1,234" -> "1234")
      normalized = s.replace(/,/g, "");
    }
  } else {
    if (s.split(".").length - 1 > 1) return null;
    normalized = s;
  }

  if (!/^(\d+(\.\d*)?|\.\d+)$/.test(normalized)) return null;
  // Tidy "12." -> "12" and ".5" -> "0.5"
  if (normalized.endsWith(".")) normalized = normalized.slice(0, -1);
  if (normalized.startsWith(".")) normalized = `0${normalized}`;
  return sign + normalized;
}

/**
 * Exactly sum dot-decimal strings (as produced by normalizeAmountInput)
 * using integer math at the maximum precision present in the inputs.
 * Returns the sum as a dot-decimal string, or null if any input is invalid.
 * No floating point is involved, so there are no rounding false-positives.
 */
export function exactDecimalSum(amounts: string[]): string | null {
  let maxScale = 0;
  const parsed: { sign: bigint; int: string; frac: string }[] = [];
  for (const a of amounts) {
    const m = /^([+-]?)(\d*)(?:\.(\d*))?$/.exec(a.trim());
    if (!m || (!m[2] && !m[3])) return null;
    parsed.push({ sign: m[1] === "-" ? -1n : 1n, int: m[2] || "0", frac: m[3] || "" });
    maxScale = Math.max(maxScale, (m[3] || "").length);
  }

  const scale = 10n ** BigInt(maxScale);
  let total = 0n;
  for (const p of parsed) {
    const units = BigInt(p.int) * scale + BigInt(p.frac.padEnd(maxScale, "0") || "0");
    total += p.sign * units;
  }

  const negative = total < 0n;
  const abs = negative ? -total : total;
  const digits = abs.toString().padStart(maxScale + 1, "0");
  const intPart = digits.slice(0, digits.length - maxScale);
  const fracPart = maxScale > 0 ? `.${digits.slice(digits.length - maxScale)}` : "";
  return `${negative ? "-" : ""}${intPart}${fracPart}`;
}

/** True when the dot-decimal string represents exactly zero. */
export function isDecimalZero(value: string): boolean {
  return /^-?0*(\.0*)?$/.test(value.trim());
}

/** Sum quantities exactly and return the result with the widest scale seen
 *  among the inputs, so "1.5" + "2.25" is "3.75", never "3.75000000000001".
 *  Falls back to "0" when nothing valid was given. */
export function sumQuantities(quantities: string[]): string {
  return exactDecimalSum(quantities) ?? "0";
}

/** Negate a dot-decimal string without going through a float. */
export function negateDecimal(value: string): string {
  const s = value.trim();
  if (s === "") return s;
  if (s.startsWith("-")) return s.slice(1);
  if (s.startsWith("+")) return `-${s.slice(1)}`;
  return `-${s}`;
}

/** Flip the sign of whatever the user has typed so far. Keeps the text as
 *  typed (grouping and comma decimals included); an empty field gets "-" so
 *  the next digits come out negative. */
export function toggleSignInput(raw: string): string {
  const s = raw.trim();
  if (s === "" || s === "-") return s === "-" ? "" : "-";
  return negateDecimal(s);
}
