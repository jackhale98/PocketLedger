import { describe, expect, it } from "vitest";
import {
  exactDecimalSum,
  isDecimalZero,
  negateDecimal,
  normalizeAmountInput,
  sumQuantities,
  toggleSignInput,
} from "../amount";

describe("normalizeAmountInput", () => {
  it("passes plain dot-decimals through", () => {
    expect(normalizeAmountInput("12.50")).toBe("12.50");
    expect(normalizeAmountInput("-3")).toBe("-3");
    expect(normalizeAmountInput("+7.1")).toBe("7.1");
  });

  it("treats a lone comma with one or two trailing digits as the decimal mark", () => {
    expect(normalizeAmountInput("1,5")).toBe("1.5");
    expect(normalizeAmountInput("1,50")).toBe("1.50");
  });

  it("treats other commas as grouping", () => {
    expect(normalizeAmountInput("1,234")).toBe("1234");
    expect(normalizeAmountInput("1,234,567")).toBe("1234567");
  });

  it("uses the last separator as the decimal mark when both appear", () => {
    expect(normalizeAmountInput("1.234,56")).toBe("1234.56");
    expect(normalizeAmountInput("1,234.56")).toBe("1234.56");
  });

  it("strips spaces and apostrophes used for grouping", () => {
    expect(normalizeAmountInput("1 234.56")).toBe("1234.56");
    expect(normalizeAmountInput("1'234.56")).toBe("1234.56");
  });

  it("tidies leading and trailing dots", () => {
    expect(normalizeAmountInput(".5")).toBe("0.5");
    expect(normalizeAmountInput("12.")).toBe("12");
  });

  it("rejects garbage", () => {
    expect(normalizeAmountInput("")).toBeNull();
    expect(normalizeAmountInput("abc")).toBeNull();
    expect(normalizeAmountInput("1.2.3")).toBeNull();
    expect(normalizeAmountInput("-")).toBeNull();
  });
});

describe("exactDecimalSum", () => {
  it("sums without float error", () => {
    expect(exactDecimalSum(["0.1", "0.2"])).toBe("0.3");
    expect(exactDecimalSum(["1.10", "2.20", "-3.30"])).toBe("0.00");
  });

  it("uses the widest scale among the inputs", () => {
    expect(exactDecimalSum(["1.5", "2.25"])).toBe("3.75");
    expect(exactDecimalSum(["1", "2"])).toBe("3");
    expect(exactDecimalSum(["-1.005", "1"])).toBe("-0.005");
  });

  it("keeps precision beyond what a double can hold", () => {
    expect(exactDecimalSum(["12345678901234567.89", "0.01"])).toBe("12345678901234567.90");
  });

  it("returns null on an invalid input", () => {
    expect(exactDecimalSum(["1", "x"])).toBeNull();
    expect(exactDecimalSum([""])).toBeNull();
  });

  it("sumQuantities falls back to zero", () => {
    expect(sumQuantities([])).toBe("0");
    expect(sumQuantities(["1", "bad"])).toBe("0");
  });
});

describe("isDecimalZero", () => {
  it("recognises zero in its many spellings", () => {
    expect(isDecimalZero("0")).toBe(true);
    expect(isDecimalZero("0.00")).toBe(true);
    expect(isDecimalZero("-0.0")).toBe(true);
    expect(isDecimalZero(".0")).toBe(true);
  });

  it("rejects non-zero", () => {
    expect(isDecimalZero("0.01")).toBe(false);
    expect(isDecimalZero("-1")).toBe(false);
  });
});

describe("sign helpers", () => {
  it("negates without parsing", () => {
    expect(negateDecimal("12.5")).toBe("-12.5");
    expect(negateDecimal("-12.5")).toBe("12.5");
    expect(negateDecimal("+3")).toBe("-3");
  });

  it("toggles what the user typed so far", () => {
    expect(toggleSignInput("")).toBe("-");
    expect(toggleSignInput("-")).toBe("");
    expect(toggleSignInput("1,5")).toBe("-1,5");
    expect(toggleSignInput("-1 234")).toBe("1 234");
  });
});
