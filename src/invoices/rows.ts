/**
 * Everything the list derives from an `InvoiceRow` before it is drawn.
 *
 * Kept out of the components so the two questions the table exists to answer
 * - "does this row need a person?" and "what order are these in?" - are
 * testable without rendering anything.
 */

import type { Cents, InvoiceRow } from "../types";
import { REVIEW_THRESHOLD } from "../types";

/**
 * The category rows carry when nothing claimed them.
 *
 * Mirrors `classify::UNCATEGORISED`. The subtlety: `db::update` writes the
 * empty string for `category: None`, so an unclassified row arrives here with
 * `category === ""` and the word 未分类 is a *label*, not a stored value.
 * Every write this module makes therefore sends `""` rather than 未分类, so
 * hand-cleared rows land in the same bucket as never-classified ones and the
 * category filter finds both.
 */
export const UNCATEGORISED = "未分类";

/** `""` → 未分类; anything else is its own name. */
export function categoryLabel(category: string): string {
  return category === "" ? UNCATEGORISED : category;
}

/**
 * 金额（不含税）.
 *
 * The row projection `db::list` returns carries only 价税合计 and 税额, so the
 * column is derived. Safe to do here and only here: both are integer 分, so
 * the subtraction is exact, and the identity 价税合计 = 金额 + 税额 is
 * checked on the Rust side - when it fails the row already carries a
 * `totalMismatch` issue and shows 待复核, which is the honest way to surface
 * a number that does not add up.
 */
export function amountExclTaxOf(row: InvoiceRow): Cents {
  return row.totalCents - row.taxCents;
}

/**
 * Whether the row needs a human.
 *
 * Mirrors the `needs_review_only` clause in `db::list` exactly - if the two
 * ever disagree, the 待复核 count in the sidebar stops matching the badges in
 * the table, and the user has no way to tell which one is lying.
 */
export function rowNeedsReview(row: InvoiceRow): boolean {
  return (
    !row.reviewed &&
    (row.minConfidence < REVIEW_THRESHOLD || row.issueCount > 0)
  );
}

export function rowIsDuplicate(row: InvoiceRow): boolean {
  return row.duplicateOf.length > 0;
}

export type SortKey = "date" | "seller" | "amount";

export interface Sort {
  key: SortKey;
  /** Descending is the default for every column: newest, largest, and - for
   *  the seller - whichever direction the user asked for last. */
  desc: boolean;
}

export const DEFAULT_SORT: Sort = { key: "date", desc: true };

/** 拼音 order for 销售方名称; a plain `<` on Chinese sorts by code point. */
const collator = new Intl.Collator("zh-CN");

/**
 * Sorts client-side rather than asking the database again.
 *
 * A personal ledger is hundreds of rows, not millions - sorting them in JS is
 * imperceptible, and it keeps the sort out of the filter, so re-sorting can
 * never disagree with the totals in the status bar (which are computed from
 * the same filter and are order-independent).
 */
export function sortRows(
  rows: readonly InvoiceRow[],
  sort: Sort,
): InvoiceRow[] {
  const direction = sort.desc ? -1 : 1;
  return [...rows].sort((a, b) => {
    let ordering: number;
    switch (sort.key) {
      case "date":
        // `YYYY-MM-DD` sorts correctly as text, which is why the whole app
        // normalises dates to it.
        ordering = a.issuedOn.localeCompare(b.issuedOn);
        break;
      case "seller":
        ordering = collator.compare(a.sellerName, b.sellerName);
        break;
      case "amount":
        ordering = a.totalCents - b.totalCents;
        break;
    }
    // Id as the tiebreak, always ascending under the primary direction, so
    // same-day rows keep import order instead of shuffling on every re-sort.
    return ordering !== 0 ? ordering * direction : (a.id - b.id) * direction;
  });
}
