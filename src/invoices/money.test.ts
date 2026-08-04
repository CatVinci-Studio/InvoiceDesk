import { describe, expect, it } from "vitest";
import { parseYuan, yuanInput } from "./money";

/**
 * Mirrors `model::tests::money_parses_the_forms_invoices_actually_use` - the
 * two parsers have to agree, or a number typed into the review form comes out
 * different from the same number read off the PDF.
 */
describe("parseYuan", () => {
  it("reads the forms people type off an invoice", () => {
    expect(parseYuan("1060.00")).toBe(106000);
    expect(parseYuan("1,060.00")).toBe(106000);
    expect(parseYuan("1，060.00")).toBe(106000);
    expect(parseYuan("¥1060")).toBe(106000);
    expect(parseYuan("￥1,234.56")).toBe(123456);
    expect(parseYuan(" 88.5 ")).toBe(8850);
    expect(parseYuan("0.05")).toBe(5);
    expect(parseYuan("-88.00")).toBe(-8800);
    expect(parseYuan(".5")).toBe(50);
  });

  /**
   * The refusals are the point of the whole module: a wrong amount that looks
   * parsed is worse than an amount the form says it cannot read.
   */
  it("refuses anything it is not sure of, rather than returning 0", () => {
    expect(parseYuan("")).toBeNull();
    expect(parseYuan("   ")).toBeNull();
    expect(parseYuan("价税合计")).toBeNull();
    expect(parseYuan("10x0")).toBeNull();
    expect(parseYuan("1.2.3")).toBeNull();
    // Two amounts run together, as a greedy regex would hand over.
    expect(parseYuan("12.34 56.78")).toBeNull();
    expect(parseYuan("-")).toBeNull();
  });

  it("rounds extra precision instead of truncating it", () => {
    expect(parseYuan("1.005")).toBe(101);
    expect(parseYuan("1.004")).toBe(100);
  });

  /** The float trap this module exists to avoid: `1.005 * 100` is
   *  100.49999999999999, which rounds the wrong way. */
  it("does not go through a binary float", () => {
    expect(parseYuan("1.005")).not.toBe(Math.round(1.005 * 100));
    expect(parseYuan("8.07")).toBe(807);
    expect(parseYuan("1234567.89")).toBe(123456789);
  });

  it("round-trips through the input format", () => {
    for (const cents of [0, 5, 807, 106000, -8800, 123456789]) {
      expect(parseYuan(yuanInput(cents))).toBe(cents);
    }
  });
});

describe("yuanInput", () => {
  it("writes a plain editable decimal, with no separators to fight", () => {
    expect(yuanInput(106000)).toBe("1060.00");
    expect(yuanInput(5)).toBe("0.05");
    expect(yuanInput(-8800)).toBe("-88.00");
    expect(yuanInput(null)).toBe("");
    expect(yuanInput(undefined)).toBe("");
  });
});
