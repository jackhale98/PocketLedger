import { afterEach, describe, expect, it } from "vitest";
import {
  amountTone,
  decimalSign,
  decimalsIn,
  formatAmount,
  formatQuantity,
  setIncognito,
} from "../format";

afterEach(() => setIncognito(false));

describe("formatQuantity", () => {
  it("groups thousands and keeps the backend's decimals", () => {
    expect(formatQuantity("1234567.80")).toBe("1,234,567.80");
    expect(formatQuantity("1200")).toBe("1,200");
    expect(formatQuantity("-1000.5")).toBe("-1,000.5");
    expect(formatQuantity("12")).toBe("12");
  });

  it("does not round-trip through a float", () => {
    expect(formatQuantity("12345678901234567890.123456789")).toBe(
      "12,345,678,901,234,567,890.123456789"
    );
    expect(formatQuantity("0.000000000000000001")).toBe("0.000000000000000001");
  });

  it("returns unparseable input unchanged", () => {
    expect(formatQuantity("n/a")).toBe("n/a");
  });

  it("masks when incognito", () => {
    setIncognito(true);
    expect(formatQuantity("1234")).toBe("•••");
    expect(formatAmount("1234", "$")).toBe("$•••");
  });
});

describe("formatAmount", () => {
  it("hugs symbol commodities and trails codes", () => {
    expect(formatAmount("10.00", "$")).toBe("$10.00");
    expect(formatAmount("10.00", "EUR")).toBe("10.00 EUR");
    expect(formatAmount("10.00", "")).toBe("10.00");
  });
});

describe("decimalsIn / decimalSign / amountTone", () => {
  it("counts decimals", () => {
    expect(decimalsIn("1.25")).toBe(2);
    expect(decimalsIn("3")).toBe(0);
  });

  it("classifies sign, zero included", () => {
    expect(decimalSign("-0.01")).toBe(-1);
    expect(decimalSign("0.00")).toBe(0);
    expect(decimalSign("-0")).toBe(0);
    expect(decimalSign("5")).toBe(1);
  });

  it("colours zero neutrally", () => {
    expect(amountTone("-1")).toBe("text-negative");
    expect(amountTone("1")).toBe("text-positive");
    expect(amountTone("0.00")).not.toMatch(/positive|negative/);
    expect(amountTone(null)).not.toMatch(/positive|negative/);
  });
});
