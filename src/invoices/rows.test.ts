import { describe, expect, it } from "vitest";
import type { InvoiceRow } from "../types";
import {
  amountExclTaxOf,
  categoryLabel,
  rowIsDuplicate,
  rowNeedsReview,
  sortRows,
} from "./rows";

function row(overrides: Partial<InvoiceRow>): InvoiceRow {
  return {
    id: 1,
    number: "24312000000012345678",
    issuedOn: "2024-03-01",
    kind: "数电票（普通）",
    sellerName: "某某酒店",
    totalCents: 106000,
    taxCents: 6000,
    category: "差旅",
    categoryRule: "住宿",
    minConfidence: 1,
    issueCount: 0,
    reviewed: false,
    sourcePath: "/tmp/a.pdf",
    duplicateOf: [],
    ...overrides,
  };
}

describe("rowNeedsReview", () => {
  /** Has to match the `needs_review_only` clause in `db::list`, or the
   *  sidebar count and the table badges start disagreeing. */
  it("flags low confidence or a validation issue, until reviewed", () => {
    expect(rowNeedsReview(row({ minConfidence: 0.75 }))).toBe(true);
    expect(rowNeedsReview(row({ issueCount: 1 }))).toBe(true);
    expect(rowNeedsReview(row({}))).toBe(false);
  });

  it("stays quiet once a person has looked at it", () => {
    expect(rowNeedsReview(row({ minConfidence: 0.5, reviewed: true }))).toBe(
      false,
    );
    expect(rowNeedsReview(row({ issueCount: 3, reviewed: true }))).toBe(false);
  });

  /** 0.9 is the threshold itself, and `<` means it passes. */
  it("treats the threshold as good enough", () => {
    expect(rowNeedsReview(row({ minConfidence: 0.9 }))).toBe(false);
    expect(rowNeedsReview(row({ minConfidence: 0.899 }))).toBe(true);
  });
});

describe("amountExclTaxOf", () => {
  it("derives 金额 from the two columns the row projection carries", () => {
    expect(amountExclTaxOf(row({ totalCents: 106000, taxCents: 6000 }))).toBe(
      100000,
    );
    // Integer cents throughout, so no rounding creeps into the column.
    expect(amountExclTaxOf(row({ totalCents: 807, taxCents: 7 }))).toBe(800);
  });
});

describe("categoryLabel", () => {
  it("shows an unclassified row as 未分类 without storing that string", () => {
    expect(categoryLabel("")).toBe("未分类");
    expect(categoryLabel("差旅")).toBe("差旅");
  });
});

describe("rowIsDuplicate", () => {
  it("is exactly 'another row shares this 发票号码'", () => {
    expect(rowIsDuplicate(row({ duplicateOf: [] }))).toBe(false);
    expect(rowIsDuplicate(row({ duplicateOf: [7] }))).toBe(true);
  });
});

describe("sortRows", () => {
  const rows = [
    row({
      id: 1,
      issuedOn: "2024-03-01",
      totalCents: 300,
      sellerName: "北京饭店",
    }),
    row({
      id: 2,
      issuedOn: "2024-01-05",
      totalCents: 900,
      sellerName: "安泰科技",
    }),
    row({
      id: 3,
      issuedOn: "2024-03-01",
      totalCents: 100,
      sellerName: "长江出租",
    }),
  ];

  it("defaults to newest first, import order within a day", () => {
    expect(
      sortRows(rows, { key: "date", desc: true }).map((r) => r.id),
    ).toEqual([3, 1, 2]);
  });

  it("reverses cleanly", () => {
    expect(
      sortRows(rows, { key: "date", desc: false }).map((r) => r.id),
    ).toEqual([2, 1, 3]);
  });

  it("sorts amounts numerically, not as text", () => {
    expect(
      sortRows(rows, { key: "amount", desc: true }).map((r) => r.id),
    ).toEqual([2, 1, 3]);
  });

  it("sorts sellers by 拼音 rather than code point", () => {
    expect(
      sortRows(rows, { key: "seller", desc: false }).map((r) => r.sellerName),
    ).toEqual(["安泰科技", "北京饭店", "长江出租"]);
  });

  it("does not mutate its input", () => {
    const before = rows.map((r) => r.id);
    sortRows(rows, { key: "amount", desc: false });
    expect(rows.map((r) => r.id)).toEqual(before);
  });
});
