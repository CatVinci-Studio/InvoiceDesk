/**
 * The row shape the 报销单 picker works in, and the pure arithmetic around it.
 *
 * The two sides of the picker arrive in two different shapes: candidates come
 * from `invoices.list` as `InvoiceRow` (the list projection), while the
 * invoices already on the sheet come from `reports.invoices` as full
 * `Invoice` records. Rendering two nearly-identical row components against
 * two nearly-identical shapes is how the two lists slowly drift apart, so
 * both are normalised into `PickedRow` at the boundary and there is exactly
 * one row renderer per side.
 *
 * Everything here is pure and free of Tauri, which is also what makes it the
 * part of this module that can be unit-tested without mocking the bridge.
 */

import { today } from "../format";
import {
  KIND_LABEL,
  type Cents,
  type Invoice,
  type InvoiceRow,
  type ReportPreview,
} from "../types";

/**
 * Mirrors `classify::UNCATEGORISED`. Duplicated rather than fetched because
 * it is a display fallback for a row whose category is empty - a round trip
 * to learn a constant that has not changed since the schema was written would
 * be silly.
 */
export const UNCATEGORISED = "未分类";

/** One invoice as either side of the picker shows it. */
export interface PickedRow {
  id: number;
  /** 发票号码, or a stand-in - a row with no number still has to be visible. */
  number: string;
  issuedOn: string;
  /** Already the Chinese label on both paths, never the enum name. */
  kind: string;
  seller: string;
  category: string;
  totalCents: Cents;
  /** How many validation problems ride on this invoice. */
  issueCount: number;
  /** Shares 发票代码+号码 with another invoice somewhere in the ledger. */
  duplicate: boolean;
}

export function rowFromInvoice(invoice: Invoice): PickedRow {
  return {
    id: invoice.id ?? -1,
    number: invoice.number.value ?? "(无号码)",
    issuedOn: invoice.issuedOn.value ?? "",
    kind: invoice.kind ? KIND_LABEL[invoice.kind] : "",
    seller: invoice.sellerName.value ?? "",
    category: invoice.category || UNCATEGORISED,
    totalCents: invoice.total.value ?? 0,
    issueCount: invoice.issues.length,
    // Not knowable from an `Invoice` alone: the duplicate check is a query
    // across the whole ledger. The builder overlays it from the preview's
    // `duplicates` list, which is the same check run one last time.
    duplicate: false,
  };
}

export function rowFromListRow(row: InvoiceRow): PickedRow {
  return {
    id: row.id,
    number: row.number || "(无号码)",
    issuedOn: row.issuedOn,
    kind: row.kind,
    seller: row.sellerName,
    category: row.category || UNCATEGORISED,
    totalCents: row.totalCents,
    issueCount: row.issueCount,
    duplicate: row.duplicateOf.length > 0,
  };
}

/**
 * Moves the row at `index` by `delta`, returning the SAME array when the move
 * would run off either end.
 *
 * Identity matters here: every reorder is persisted with
 * `reports.setInvoices`, and returning a fresh array for a no-op would send a
 * pointless write on every click of a disabled-looking button.
 */
export function moveRow(
  rows: PickedRow[],
  index: number,
  delta: number,
): PickedRow[] {
  const target = index + delta;
  if (index < 0 || index >= rows.length) return rows;
  if (target < 0 || target >= rows.length) return rows;
  const next = rows.slice();
  const [row] = next.splice(index, 1);
  next.splice(target, 0, row);
  return next;
}

/**
 * Oldest first, which is the order a 明细表 is read and checked in. Rows with
 * no date sink to the bottom rather than sorting as the empty string, where
 * they would head the table and look like the earliest expenses.
 */
export function sortByDate(rows: PickedRow[]): PickedRow[] {
  return rows.slice().sort((a, b) => {
    if (!a.issuedOn && !b.issuedOn) return 0;
    if (!a.issuedOn) return 1;
    if (!b.issuedOn) return -1;
    return a.issuedOn.localeCompare(b.issuedOn);
  });
}

/** Cents in, cents out - see the note on `Cents`. */
export function sumCents(rows: PickedRow[]): Cents {
  return rows.reduce((sum, row) => sum + row.totalCents, 0);
}

export function shareOf(part: Cents, whole: Cents): number {
  return whole === 0 ? 0 : part / whole;
}

export function formatShare(share: number): string {
  return `${(share * 100).toFixed(1)}%`;
}

/** `2026年8月报销单` - what the sheet is called nine times out of ten. */
export function defaultReportTitle(date: string = today()): string {
  const [year, month] = date.split("-");
  if (!year || !month) return "报销单";
  return `${year}年${Number(month)}月报销单`;
}

/**
 * A title turned into something a file system will accept.
 *
 * The title is free text and routinely contains a slash ("差旅/招待"), which
 * the save dialog would either reject or silently read as a directory - so it
 * is stripped before being offered as the default file name.
 */
export function safeFileName(title: string): string {
  const cleaned = title.replace(/[\\/:*?"<>|]/g, "").trim();
  return cleaned || "报销单";
}

/** The last path segment, for showing a chosen template without its folder. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/**
 * What the preview looks like before the first one has come back.
 *
 * Zeroes rather than a nullable preview: every figure on screen is derived
 * from this, and threading `null` through all of them buys nothing but
 * question marks - the panel is visibly loading at that moment anyway.
 */
export const EMPTY_PREVIEW: ReportPreview = {
  count: 0,
  amountExclTaxCents: 0,
  taxCents: 0,
  totalCents: 0,
  deductibleTaxCents: 0,
  capitalAmount: "零圆整",
  byCategory: [],
  warnings: [],
  duplicates: [],
};
