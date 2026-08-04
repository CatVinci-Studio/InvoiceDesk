/**
 * The picker's arithmetic, tested where it is pure.
 *
 * The component itself is not worth a render test - it is mostly wiring to
 * commands that only exist inside Tauri - but the row normalisation and the
 * reordering ARE worth one, because they decide what ends up in
 * `setInvoices`, and `setInvoices` decides the row order of the sheet that
 * goes to finance.
 */

import { describe, expect, it } from "vitest";
import {
  emptyField,
  manualField,
  type Invoice,
  type InvoiceRow,
} from "../types";
import {
  UNCATEGORISED,
  baseName,
  defaultReportTitle,
  formatShare,
  moveRow,
  rowFromInvoice,
  rowFromListRow,
  safeFileName,
  shareOf,
  sortByDate,
  sumCents,
  type PickedRow,
} from "./rows";

function invoice(overrides: Partial<Invoice> = {}): Invoice {
  return {
    id: 1,
    kind: "digitalGeneral",
    number: manualField("24312000000012345678"),
    code: emptyField<string>(),
    issuedOn: manualField("2024-03-01"),
    checkCode: emptyField<string>(),
    buyerName: emptyField<string>(),
    buyerTaxId: emptyField<string>(),
    sellerName: manualField("某某酒店"),
    sellerTaxId: emptyField<string>(),
    amountExclTax: manualField(100_000),
    tax: manualField(6_000),
    total: manualField(106_000),
    items: [],
    remark: emptyField<string>(),
    category: "住宿",
    categoryRule: null,
    sourcePath: "/tmp/a.pdf",
    fileHash: "hash",
    issues: [],
    ...overrides,
  };
}

function listRow(overrides: Partial<InvoiceRow> = {}): InvoiceRow {
  return {
    id: 2,
    number: "24312000000087654321",
    issuedOn: "2024-03-05",
    kind: "铁路电子客票",
    sellerName: "中国铁路",
    totalCents: 55_350,
    taxCents: 0,
    category: "长途交通",
    categoryRule: "",
    minConfidence: 1,
    issueCount: 0,
    reviewed: true,
    sourcePath: "/tmp/b.pdf",
    duplicateOf: [],
    ...overrides,
  };
}

function picked(id: number, issuedOn: string, totalCents = 100): PickedRow {
  return {
    id,
    number: `no-${id}`,
    issuedOn,
    kind: "增值税普通发票",
    seller: `卖方${id}`,
    category: "餐饮",
    totalCents,
    issueCount: 0,
    duplicate: false,
  };
}

describe("row normalisation", () => {
  it("turns an invoice's fields into flat display values", () => {
    const row = rowFromInvoice(invoice());
    expect(row).toMatchObject({
      id: 1,
      number: "24312000000012345678",
      issuedOn: "2024-03-01",
      kind: "数电票（普通）",
      seller: "某某酒店",
      category: "住宿",
      totalCents: 106_000,
    });
  });

  /** A half-read invoice still has to be visible and countable in the picker. */
  it("keeps an unreadable invoice on screen rather than blanking it", () => {
    const row = rowFromInvoice(
      invoice({
        number: emptyField<string>(),
        total: emptyField<number>(),
        category: null,
        kind: null,
      }),
    );
    expect(row.number).toBe("(无号码)");
    expect(row.totalCents).toBe(0);
    expect(row.category).toBe(UNCATEGORISED);
    expect(row.kind).toBe("");
  });

  it("flags a candidate that shares its number with another invoice", () => {
    expect(rowFromListRow(listRow()).duplicate).toBe(false);
    expect(rowFromListRow(listRow({ duplicateOf: [7] })).duplicate).toBe(true);
  });
});

describe("ordering", () => {
  it("moves a row and leaves the rest in order", () => {
    const rows = [
      picked(1, "2024-03-01"),
      picked(2, "2024-03-02"),
      picked(3, "2024-03-03"),
    ];
    expect(moveRow(rows, 2, -1).map((r) => r.id)).toEqual([1, 3, 2]);
    expect(moveRow(rows, 0, 1).map((r) => r.id)).toEqual([2, 1, 3]);
  });

  /** No-ops return the same array so nothing is persisted for them. */
  it("refuses to move past either end", () => {
    const rows = [picked(1, "2024-03-01"), picked(2, "2024-03-02")];
    expect(moveRow(rows, 0, -1)).toBe(rows);
    expect(moveRow(rows, 1, 1)).toBe(rows);
    expect(moveRow(rows, 9, -1)).toBe(rows);
  });

  it("sorts oldest first and sinks undated rows to the bottom", () => {
    const rows = [
      picked(1, "2024-03-09"),
      picked(2, ""),
      picked(3, "2024-03-02"),
    ];
    expect(sortByDate(rows).map((r) => r.id)).toEqual([3, 1, 2]);
  });
});

describe("totals", () => {
  it("adds cents, never yuan", () => {
    expect(sumCents([picked(1, "", 1), picked(2, "", 2)])).toBe(3);
    expect(sumCents([])).toBe(0);
  });

  it("does not divide by a zero total", () => {
    expect(shareOf(0, 0)).toBe(0);
    expect(formatShare(shareOf(2_500, 10_000))).toBe("25.0%");
  });
});

describe("naming", () => {
  it("names a new sheet after the current month", () => {
    expect(defaultReportTitle("2026-08-03")).toBe("2026年8月报销单");
  });

  /** A title like "差旅/招待" must not be read as a path by the save dialog. */
  it("strips path characters out of a suggested file name", () => {
    expect(safeFileName("8月 差旅/招待")).toBe("8月 差旅招待");
    expect(safeFileName("///")).toBe("报销单");
  });

  it("shows a template file without its folder", () => {
    expect(baseName("/Users/a/报销单模板.xlsx")).toBe("报销单模板.xlsx");
    expect(baseName("C:\\forms\\报销单模板.xlsx")).toBe("报销单模板.xlsx");
  });
});
