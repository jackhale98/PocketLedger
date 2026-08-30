import type { AmountEntry, PostingSummary } from "../../api/types";

/** Every amount on a posting. Newer backends send `amounts`; older ones only
 *  the first amount, which is folded into the same shape. An elided posting
 *  yields nothing. */
export function postingAmounts(posting: PostingSummary): AmountEntry[] {
  if (posting.amounts && posting.amounts.length > 0) return posting.amounts;
  if (posting.amount === null) return [];
  return [{ quantity: posting.amount, commodity: posting.commodity ?? "" }];
}

const BALANCE_ACCOUNT = /^(assets|liabilities)(:|$)/i;

/** The posting whose amount best summarises a transaction in a list: the
 *  first asset or liability posting with an amount (the bank side), else the
 *  first posting that has an amount at all. Null only when nothing does. */
export function headlinePosting(postings: PostingSummary[]): PostingSummary | null {
  const withAmount = postings.filter((p) => postingAmounts(p).length > 0);
  return (
    withAmount.find((p) => BALANCE_ACCOUNT.test(p.account)) ??
    withAmount[0] ??
    null
  );
}
