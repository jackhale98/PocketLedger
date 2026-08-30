import { describe, expect, it } from "vitest";
import {
  collapsibleAccounts,
  hasChildren,
  isHiddenUnder,
  isRevealedBy,
  parentAccounts,
  toggleCollapsed,
} from "../tree";

const rows = [
  { account: "assets" },
  { account: "assets:bank" },
  { account: "assets:bank:checking" },
  { account: "assets:cash" },
  { account: "expenses" },
  { account: "expenses:food" },
];

describe("parentAccounts", () => {
  it("finds every account with a descendant row", () => {
    expect([...parentAccounts(rows)].sort()).toEqual(["assets", "assets:bank", "expenses"]);
    expect(collapsibleAccounts(rows)).toEqual(parentAccounts(rows));
  });

  it("ignores ancestors that are not rows themselves", () => {
    expect([...parentAccounts([{ account: "a:b:c" }, { account: "a:b" }])]).toEqual(["a:b"]);
  });

  it("agrees with hasChildren", () => {
    const parents = parentAccounts(rows);
    for (const r of rows) {
      expect(parents.has(r.account)).toBe(hasChildren(rows, r.account));
    }
  });
});

describe("visibility", () => {
  it("hides descendants of collapsed accounts, not the account itself", () => {
    const collapsed = new Set(["assets:bank"]);
    expect(isHiddenUnder(collapsed, "assets:bank")).toBe(false);
    expect(isHiddenUnder(collapsed, "assets:bank:checking")).toBe(true);
    expect(isHiddenUnder(collapsed, "assets:cash")).toBe(false);
  });

  it("reveals only when every ancestor is expanded", () => {
    const expanded = new Set(["assets"]);
    expect(isRevealedBy(expanded, "assets")).toBe(true);
    expect(isRevealedBy(expanded, "assets:bank")).toBe(true);
    expect(isRevealedBy(expanded, "assets:bank:checking")).toBe(false);
  });

  it("toggles without mutating", () => {
    const a = new Set<string>();
    const b = toggleCollapsed(a, "x");
    expect(a.has("x")).toBe(false);
    expect(b.has("x")).toBe(true);
    expect(toggleCollapsed(b, "x").has("x")).toBe(false);
  });
});
