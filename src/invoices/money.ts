/**
 * 元 ⇄ 分, for the review form and nowhere else.
 *
 * Everything below the UI counts in `Cents` (see the note on `Cents` in
 * `types.ts` and on `Money` in `model.rs`), but a human types 元 - "1060.00"
 * off the face of the invoice. That one conversion is the only place the two
 * units meet, so it lives in one file with tests around it.
 *
 * Two rules it exists to enforce:
 *
 * 1. **Never `Math.round(parseFloat(text) * 100)`.** Binary floats cannot
 *    represent 0.1, so `1.005 * 100` is 100.49999999999999 and rounds DOWN to
 *    ¥1.00 - a silent one-fen error in a number somebody will reconcile
 *    against a bank statement. Parsing the yuan part and the fraction part as
 *    separate integers has no such failure mode.
 *
 * 2. **Refuse rather than return 0.** A zero that looks parsed shrinks a
 *    reimbursement total without anyone noticing; an input the form marks as
 *    unreadable does not. `parseYuan` returns null for anything it is not
 *    sure of, and the detail drawer refuses to save while a money field is in
 *    that state. This mirrors `Money::parse` on the Rust side, which returns
 *    `Option<Money>` for exactly the same reason.
 */

import type { Cents } from "../types";

/**
 * A bare decimal number, optionally signed. Deliberately stricter than
 * `Money::parse`, which filters out non-digits and parses what is left: that
 * behaviour is right for a regex hit scraped out of a PDF (where "（小写）"
 * and stray glyphs ride along), and wrong for a form field, where "10x0" is a
 * typo the user wants to be told about rather than silently read as 100.
 */
const DECIMAL = /^-?(?:\d+(?:\.\d*)?|\.\d+)$/;

/**
 * `"1,060.00"` → `106000`; `""`, `"abc"`, `"1.2.3"` → null.
 *
 * Currency marks, thousands separators (both ASCII and full-width, because a
 * Chinese IME produces `，`) and surrounding whitespace are stripped, since
 * those are decoration on a number rather than a different number. Anything
 * else that survives makes the whole input unreadable.
 */
export function parseYuan(raw: string): Cents | null {
  const cleaned = raw.replace(/[¥￥,，\s]/g, "");
  if (cleaned === "" || !DECIMAL.test(cleaned)) return null;

  const negative = cleaned.startsWith("-");
  const digits = negative ? cleaned.slice(1) : cleaned;
  const dot = digits.indexOf(".");
  const yuanPart = dot === -1 ? digits : digits.slice(0, dot);
  const fracPart = dot === -1 ? "" : digits.slice(dot + 1);

  const yuan = yuanPart === "" ? 0 : Number(yuanPart);
  if (!Number.isSafeInteger(yuan)) return null;

  // Round at 分 rather than truncating, matching `Money::parse`: line items
  // occasionally carry more precision than the total does, and truncating
  // would make the items stop summing to it.
  let fraction: number;
  if (fracPart.length === 0) fraction = 0;
  else if (fracPart.length === 1) fraction = Number(fracPart) * 10;
  else if (fracPart.length === 2) fraction = Number(fracPart);
  else
    fraction =
      Number(fracPart.slice(0, 2)) + (fracPart.charCodeAt(2) >= 53 ? 1 : 0);

  const total = yuan * 100 + fraction;
  if (!Number.isSafeInteger(total)) return null;
  return negative ? -total : total;
}

/**
 * `106000` → `"1060.00"`, for seeding an editable field.
 *
 * No thousands separators and no ¥: the string goes back through `parseYuan`
 * on the next keystroke, and a field that reformats itself while you are
 * typing in it is maddening. `formatMoney` stays the display path.
 */
export function yuanInput(cents: Cents | null | undefined): string {
  if (cents === null || cents === undefined) return "";
  const sign = cents < 0 ? "-" : "";
  const absolute = Math.abs(cents);
  return `${sign}${Math.floor(absolute / 100)}.${String(absolute % 100).padStart(2, "0")}`;
}
