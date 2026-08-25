/** Helpers for account-tree rows, which arrive flat with a depth and a full
 *  colon-separated account name rather than as nested structures. */

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
  return new Set(rows.filter((r) => hasChildren(rows, r.account)).map((r) => r.account));
}
