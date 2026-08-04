/**
 * Turning a `Rule` into the sentences the pane shows, and back.
 *
 * All of it is pure and lives away from the components on purpose: the whole
 * value of this pane is that a rule reads as a reason a person can check
 * ("为什么这张发票被归到餐饮？"), and prose that carries that much weight
 * deserves tests that do not need a DOM. The classifier's semantics are
 * mirrored here in Chinese - keywords inside one condition are alternatives,
 * conditions inside one rule are all required - so if `classify/mod.rs` ever
 * changes its mind, this file is the one place the wording has to follow.
 */

import {
  MATCH_FIELD_LABEL,
  type Condition,
  type MatchField,
  type Rule,
} from "../types";

// The three connectives the whole pane speaks. Exported rather than inlined so
// the prose built as a string (titles, tests, the auto-generated rule name) and
// the prose built as JSX (the list rows, where keywords get their own styling)
// can never drift apart.

/** `Condition::matches` is a substring test, so 包含 is literally accurate. */
export const CONTAINS = "包含";
/** Between the keywords of ONE condition - any single hit satisfies it. */
export const KEYWORD_JOINER = "或";
/** Between the conditions of ONE rule - every one of them must hold. */
export const CONDITION_JOINER = "并且";

/** `住宿服务` / `旅客运输服务 或 运输服务`. */
export function describeKeywords(keywords: string[]): string {
  return keywords.join(` ${KEYWORD_JOINER} `);
}

/** `税收分类简称 包含 住宿服务`. */
export function describeCondition(condition: Condition): string {
  const label = MATCH_FIELD_LABEL[condition.field];
  if (condition.keywords.length === 0) return `${label} 未填写关键词`;
  return `${label} ${CONTAINS} ${describeKeywords(condition.keywords)}`;
}

/** Every condition of a rule, joined by 并且. */
export function describeRule(rule: Rule): string {
  if (rule.conditions.length === 0) return "没有条件，不会匹配任何发票";
  return rule.conditions.map(describeCondition).join(` ${CONDITION_JOINER} `);
}

/**
 * The name a rule gets if the user does not type one.
 *
 * Deliberately the same shape `defaults::rules()` builds on the Rust side
 * (`税收分类简称：住宿服务`), so a rule the user adds by hand is
 * indistinguishable from a built-in in the 「为什么是这个类别」 note on an
 * invoice - which is where this string actually ends up.
 *
 * Only the first condition makes it into the name. A name that spelled out
 * three ANDed conditions would be too long for the invoice row it has to fit
 * in, and the editor shows the full prose anyway.
 */
export function defaultRuleName(conditions: Condition[]): string {
  const [first, ...rest] = conditions;
  if (!first || first.keywords.length === 0) return "";
  const base = `${MATCH_FIELD_LABEL[first.field]}：${first.keywords.join("、")}`;
  return rest.length > 0 ? `${base} 等 ${conditions.length} 项条件` : base;
}

/** True when the stored name is exactly what we would have generated. */
export function isAutoName(rule: Rule): boolean {
  return rule.name.trim() === defaultRuleName(rule.conditions);
}

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

export interface PriorityBand {
  value: number;
  label: string;
  hint: string;
}

/**
 * The bands `classify/defaults.rs` lays its seed rules out in.
 *
 * Shown in the editor next to the number field rather than left implicit,
 * because a bare 「优先级」 box invites 9999 - a user with no other information
 * reasonably reads the field as "how much do I mean it". The number only ever
 * matters *relative to the other rules*, and the only way to make that visible
 * is to show what the numbers already in the database mean. Once the bands are
 * on screen, "this is a seller-name guess, so 50" is an obvious choice and
 * 9999 stops looking like one.
 *
 * Kept in sync by hand with `defaults.rs`; the 120 band is not in the doc
 * table there but is what the 项目名称 seeds actually use, and a user editing
 * one of those rules would otherwise see an unexplained number.
 */
export const PRIORITY_BANDS: PriorityBand[] = [
  {
    value: 200,
    label: "AI 建议采纳",
    hint: "接受 AI 建议时自动生成，压过全部内置规则",
  },
  { value: 150, label: "税收分类简称", hint: "税局自己的分类，最可靠" },
  { value: 120, label: "项目名称", hint: "分类简称不够细时的补充" },
  { value: 100, label: "票种", hint: "火车票、行程单这类票据本身就说明了用途" },
  { value: 50, label: "销售方兜底", hint: "最容易误伤，排最后" },
];

/** The highest band, used to flag numbers that outrank everything shipped. */
const TOP_BAND = PRIORITY_BANDS[0];

/**
 * Where a number sits among the bands, as a sentence.
 *
 * `warn` is not "this is wrong" - a user genuinely may want to override
 * everything for one vendor. It marks the case worth a second look: a rule
 * above 200 beats the 税收分类简称 rules, which are the most reliable signal
 * on an invoice, so it had better be more specific than they are.
 */
export function priorityNote(priority: number): {
  text: string;
  warn: boolean;
} {
  const exact = PRIORITY_BANDS.find((band) => band.value === priority);
  if (exact) {
    return {
      text: `与内置的「${exact.label}」同档：${exact.hint}`,
      warn: false,
    };
  }
  if (priority > TOP_BAND.value) {
    return {
      text: `高于全部内置规则（最高 ${TOP_BAND.value}），会盖过更可靠的税收分类简称规则`,
      warn: true,
    };
  }
  const above = [...PRIORITY_BANDS]
    .reverse()
    .find((band) => band.value > priority);
  const below = PRIORITY_BANDS.find((band) => band.value < priority);
  if (above && below) {
    return {
      text: `介于「${below.label}」(${below.value}) 和「${above.label}」(${above.value}) 之间`,
      warn: false,
    };
  }
  return {
    text: "低于全部内置规则，只有在没有别的规则命中时才轮得到它",
    warn: false,
  };
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/**
 * Why this draft cannot be saved, or `null` if it can.
 *
 * The two structural checks are not cosmetic - `Rule::matches` refuses a rule
 * with no conditions (it would otherwise claim every invoice ever imported),
 * and a condition with no keywords can never be true, so it silently poisons
 * the whole AND. Both would look to the user like the app forgot their rule,
 * which is the worst possible failure for a pane whose entire job is to be
 * explainable. So they are blocked at the point of saving, with the reason
 * spelled out.
 */
export function validateDraft(draft: Rule): string | null {
  if (!draft.category.trim()) return "请先选择或填写费用类别";
  if (draft.conditions.length === 0) {
    return "至少要有一个条件：没有条件的规则会匹配所有发票，因此会被直接忽略";
  }
  if (draft.conditions.some((condition) => condition.keywords.length === 0)) {
    return "每个条件都要至少填一个关键词，否则这条规则永远不会命中";
  }
  if (!Number.isInteger(draft.priority)) return "优先级必须是整数";
  return null;
}

// ---------------------------------------------------------------------------
// List shape
// ---------------------------------------------------------------------------

export interface RuleGroup {
  category: string;
  rules: Rule[];
}

/** Mirrors `classify::UNCATEGORISED`. */
export const UNCATEGORISED = "未分类";

const COLLATOR = new Intl.Collator("zh-Hans-CN");

/**
 * The rules as the list shows them: grouped by 费用类别, and inside a group
 * ordered exactly the way `classify()` resolves them - priority descending,
 * ties broken by the lower id.
 *
 * Matching the classifier's own ordering is the point. Read top to bottom, a
 * group is then literally the sequence the app tries, so "why did this land in
 * 餐饮 rather than 住宿" is answered by reading down until the first line that
 * describes the invoice.
 */
export function groupByCategory(rules: Rule[]): RuleGroup[] {
  const byCategory = new Map<string, Rule[]>();
  for (const rule of rules) {
    const bucket = byCategory.get(rule.category);
    if (bucket) bucket.push(rule);
    else byCategory.set(rule.category, [rule]);
  }

  return (
    [...byCategory.entries()]
      .map(([category, group]) => ({
        category,
        rules: [...group].sort(
          (a, b) =>
            b.priority - a.priority ||
            (a.id ?? Number.MAX_SAFE_INTEGER) -
              (b.id ?? Number.MAX_SAFE_INTEGER),
        ),
      }))
      // 未分类 last: it is the classifier's fallback rather than a category
      // anyone deliberately files things under, so it belongs at the bottom of
      // the pane the way it belongs at the bottom of the report.
      .sort((a, b) => {
        if (a.category === UNCATEGORISED) return 1;
        if (b.category === UNCATEGORISED) return -1;
        return COLLATOR.compare(a.category, b.category);
      })
  );
}

/** Every category currently spoken for by a rule, plus 未分类. */
export function categoriesOf(rules: Rule[]): string[] {
  const seen = new Set(rules.map((rule) => rule.category).filter(Boolean));
  seen.add(UNCATEGORISED);
  return [...seen].sort(COLLATOR.compare);
}

// ---------------------------------------------------------------------------
// Drafts
// ---------------------------------------------------------------------------

export function emptyCondition(field: MatchField = "taxCategory"): Condition {
  return { field, keywords: [] };
}

/**
 * A blank rule for the 新建 button.
 *
 * Defaults to a 税收分类简称 condition at 150 because that is the rule worth
 * writing: it is the tax authority's own classification, so it generalises to
 * every other vendor in the same trade, whereas the seller-name rule the user
 * would otherwise reach for helps with exactly one company. Starting the form
 * there is a cheap way to push the good habit.
 */
export function blankRule(category = ""): Rule {
  return {
    id: null,
    name: "",
    category,
    priority: 150,
    enabled: true,
    conditions: [emptyCondition()],
  };
}
