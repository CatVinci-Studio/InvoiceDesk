/**
 * Display formatting. Every function here is presentation only - nothing in
 * this file is ever used to compute a total.
 */

import type { Cents, Field, Invoice, ValidationIssue } from "./types";
import { REVIEW_THRESHOLD } from "./types";

/**
 * `¥1,234.56`.
 *
 * The division by 100 happens HERE and only here, at the last possible
 * moment before the number becomes text - see the note on `Cents`.
 */
export function formatMoney(cents: Cents | null | undefined): string {
  if (cents === null || cents === undefined) return "—";
  const negative = cents < 0;
  const absolute = Math.abs(cents);
  const yuan = Math.floor(absolute / 100).toLocaleString("zh-CN");
  const fraction = String(absolute % 100).padStart(2, "0");
  return `${negative ? "-" : ""}¥${yuan}.${fraction}`;
}

/** `¥1,234.56` without the symbol, for right-aligned table columns. */
export function formatAmount(cents: Cents | null | undefined): string {
  return formatMoney(cents).replace("¥", "");
}

/** `2024-03-01` → `2024/03/01`; anything unparseable passes through. */
export function formatDate(date: string | null | undefined): string {
  if (!date) return "—";
  return date.replaceAll("-", "/");
}

/** `2024-03-01` → `3月`, for grouping headers. */
export function monthLabel(date: string): string {
  const [, month] = date.split("-");
  return month ? `${Number(month)}月` : date;
}

/** Today as `YYYY-MM-DD`, in the local timezone. */
export function today(): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/** The first day of the month `back` months ago, as `YYYY-MM-DD`. */
export function monthsAgo(back: number): string {
  const now = new Date();
  const date = new Date(now.getFullYear(), now.getMonth() - back, 1);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-01`;
}

/**
 * How a field's trustworthiness reads on screen.
 *
 * Three levels, not a percentage: a number like "87%" invites the user to
 * reason about a figure that is really just a source ranking, and the only
 * decision it feeds is binary - look at this myself, or not.
 */
export type Trust = "ok" | "review" | "missing";

export function trustOf<T>(field: Field<T>): Trust {
  if (field.value === null) return "missing";
  return field.confidence < REVIEW_THRESHOLD ? "review" : "ok";
}

export const TRUST_LABEL: Record<Trust, string> = {
  ok: "已确认",
  review: "待复核",
  missing: "未识别",
};

/** Reader-facing text for a validation issue. Mirrors `ValidationIssue::message`. */
export function issueMessage(issue: ValidationIssue): string {
  switch (issue.kind) {
    case "totalMismatch":
      return `价税合计对不上：金额+税额=${formatMoney(
        issue.detail.expected,
      )}，票面${formatMoney(issue.detail.found)}`;
    case "numberLength":
      return `发票号码位数异常：${issue.detail.found} 位，应为 ${issue.detail.expected} 位`;
    case "badDate":
      return `开票日期无法识别：${issue.detail.raw}`;
    case "futureDate":
      return `开票日期晚于今天：${issue.detail.date}`;
    case "missingField":
      return `缺少${issue.detail.field}`;
  }
}

/** Hard error (red) vs caution (amber). Mirrors `ValidationIssue::is_error`. */
export function isError(issue: ValidationIssue): boolean {
  return issue.kind === "totalMismatch" || issue.kind === "badDate";
}

/** The fields the review pane highlights. Mirrors `fields_needing_review`. */
export function fieldsNeedingReview(invoice: Invoice): string[] {
  const out: string[] = [];
  const check = (field: Field<unknown>, name: string) => {
    if (field.value === null || field.confidence < REVIEW_THRESHOLD)
      out.push(name);
  };
  check(invoice.number, "发票号码");
  check(invoice.issuedOn, "开票日期");
  check(invoice.sellerName, "销售方名称");
  check(invoice.total, "价税合计");
  return out;
}

/** `12` → `12 张`. Chinese counts read badly without their measure word. */
export function countLabel(count: number): string {
  return `${count} 张`;
}
