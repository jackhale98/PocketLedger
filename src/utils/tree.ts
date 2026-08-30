/** Helpers for account-tree rows, which arrive flat with a depth and a full
 *  colon-separated account name rather than as nested structures. */

/** The set of accounts in `rows` that have at least one descendant row.
 *  Built in one pass over the rows -- every ancestor of every row -- so the
 *  per-row check below is a set lookup, not a scan. */
export function parentAccounts(rows: { account: string }[]): Set<string> {
  const present = new Set(rows.map((r) => r.account));
  const parents = new Set<string>();
  for (const r of rows) {
    const parts = r.account.split(":");
    for (let i = parts.length - 1; i >= 1; i--) {
      const ancestor = parts.slice(0, i).join(":");
      if (present.has(ancestor)) parents.add(ancestor);
    }
  }
  return parents;
}

/** True when some row is a descendant of `account`. */
export function hasChildren(
  rows: { account: string }[],
  account: string
): boolean {
  const prefix = `${account}:`;
  return rows.some((r) => r.account.startsWith(prefix));
}

/** True when any ancestor of `account` is collapsed, so the row is hidden.
 *  The row itself being collapsed doesn't hide it — only its descendants. */
export function isHiddenUnder(
  collapsed: Set<string>,
  account: string
): boolean {
  if (collapsed.size === 0) return false;
  const parts = account.split(":");
  for (let i = 1; i < parts.length; i++) {
    if (collapsed.has(parts.slice(0, i).join(":"))) return true;
  }
  return false;
}

/** True when every ancestor of `account` is in `expanded`, so the row shows
 *  in an expand-to-reveal tree. A top-level account has no ancestors. */
export function isRevealedBy(
  expanded: Set<string>,
  account: string
): boolean {
  const parts = account.split(":");
  for (let i = 1; i < parts.length; i++) {
    if (!expanded.has(parts.slice(0, i).join(":"))) return false;
  }
  return true;
}

export function toggleCollapsed(
  collapsed: Set<string>,
  account: string
): Set<string> {
  const next = new Set(collapsed);
  if (next.has(account)) {
    next.delete(account);
  } else {
    next.add(account);
  }
  return next;
}

/** Every account in `rows` that has descendants — the set to collapse when
 *  collapsing everything. */
export function collapsibleAccounts(rows: { account: string }[]): Set<string> {
  return parentAccounts(rows);
}
