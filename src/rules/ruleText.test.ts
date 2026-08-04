import { describe, expect, it } from "vitest";
import type { Rule } from "../types";
import {
  categoriesOf,
  defaultRuleName,
  describeCondition,
  describeRule,
  groupByCategory,
  isAutoName,
  priorityNote,
  validateDraft,
} from "./ruleText";

function rule(overrides: Partial<Rule> = {}): Rule {
  return {
    id: 1,
    name: "税收分类简称：住宿服务",
    category: "住宿",
    priority: 150,
    enabled: true,
    conditions: [{ field: "taxCategory", keywords: ["住宿服务"] }],
    ...overrides,
  };
}

describe("prose", () => {
  it("reads a condition as a sentence rather than as a field name", () => {
    expect(
      describeCondition({ field: "taxCategory", keywords: ["住宿服务"] }),
    ).toBe("税收分类简称 包含 住宿服务");
  });

  /** Keywords are alternatives - `Condition::matches` uses `any`. */
  it("joins several keywords with 或", () => {
    expect(
      describeCondition({
        field: "itemName",
        keywords: ["高铁", "动车", "机票"],
      }),
    ).toBe("项目名称 包含 高铁 或 动车 或 机票");
  });

  /** Conditions are conjunctive - `Rule::matches` uses `all`. */
  it("joins several conditions with 并且", () => {
    expect(
      describeRule(
        rule({
          conditions: [
            { field: "taxCategory", keywords: ["住宿服务"] },
            { field: "sellerName", keywords: ["航空"] },
          ],
        }),
      ),
    ).toBe("税收分类简称 包含 住宿服务 并且 销售方名称 包含 航空");
  });

  it("says so when a rule has no conditions at all", () => {
    expect(describeRule(rule({ conditions: [] }))).toContain("不会匹配");
  });
});

describe("names", () => {
  /** Same shape as `defaults::rules()`, so a hand-made rule and a built-in
   *  read identically in the 「为什么是这个类别」 note on an invoice. */
  it("generates the same name the Rust defaults do", () => {
    expect(
      defaultRuleName([{ field: "taxCategory", keywords: ["住宿服务"] }]),
    ).toBe("税收分类简称：住宿服务");
    expect(
      defaultRuleName([
        { field: "sellerName", keywords: ["酒店", "宾馆", "旅馆"] },
      ]),
    ).toBe("销售方名称：酒店、宾馆、旅馆");
  });

  it("keeps the name short when a rule has several conditions", () => {
    expect(
      defaultRuleName([
        { field: "taxCategory", keywords: ["住宿服务"] },
        { field: "sellerName", keywords: ["航空"] },
      ]),
    ).toBe("税收分类简称：住宿服务 等 2 项条件");
  });

  it("has nothing to generate from before a keyword is typed", () => {
    expect(defaultRuleName([{ field: "taxCategory", keywords: [] }])).toBe("");
    expect(defaultRuleName([])).toBe("");
  });

  it("recognises a name it generated itself, so editing conditions updates it", () => {
    expect(isAutoName(rule())).toBe(true);
    expect(isAutoName(rule({ name: "住宿（含长包房）" }))).toBe(false);
  });
});

describe("priority bands", () => {
  it("names the band when the number is one of them", () => {
    expect(priorityNote(150).text).toContain("税收分类简称");
    expect(priorityNote(150).warn).toBe(false);
  });

  /** The 9999 case the bands exist to prevent - flagged, not forbidden. */
  it("warns about a number that outranks the tax-authority rules", () => {
    const note = priorityNote(9999);
    expect(note.warn).toBe(true);
    expect(note.text).toContain("税收分类简称");
  });

  it("places a number between the two bands it falls between", () => {
    const note = priorityNote(130);
    expect(note.warn).toBe(false);
    expect(note.text).toContain("项目名称");
    expect(note.text).toContain("税收分类简称");
  });

  it("explains that a very low number only acts as a last resort", () => {
    expect(priorityNote(1).text).toContain("没有别的规则命中");
  });
});

describe("validation", () => {
  it("accepts a well-formed rule", () => {
    expect(validateDraft(rule())).toBeNull();
  });

  /** `Rule::matches` refuses these, so saving one would look like data loss. */
  it("refuses a rule with no conditions", () => {
    expect(validateDraft(rule({ conditions: [] }))).toContain(
      "至少要有一个条件",
    );
  });

  it("refuses a condition with no keywords, which can never be true", () => {
    expect(
      validateDraft(rule({ conditions: [{ field: "remark", keywords: [] }] })),
    ).toContain("关键词");
  });

  it("refuses a rule with no category", () => {
    expect(validateDraft(rule({ category: "  " }))).toContain("费用类别");
  });

  it("refuses a non-integer priority", () => {
    expect(validateDraft(rule({ priority: Number.NaN }))).toContain("整数");
  });
});

describe("grouping", () => {
  /** The list must read in the order `classify()` resolves rules, or reading
   *  down a group would not answer "why this one and not that one". */
  it("orders a group by priority, then by the id that breaks the tie", () => {
    const groups = groupByCategory([
      rule({ id: 3, category: "餐饮", priority: 50 }),
      rule({ id: 2, category: "餐饮", priority: 150 }),
      rule({ id: 1, category: "餐饮", priority: 150 }),
    ]);
    expect(groups[0].rules.map((r) => r.id)).toEqual([1, 2, 3]);
  });

  it("pins 未分类 to the bottom", () => {
    const groups = groupByCategory([
      rule({ id: 1, category: "未分类" }),
      rule({ id: 2, category: "住宿" }),
      rule({ id: 3, category: "餐饮" }),
    ]);
    expect(groups[groups.length - 1].category).toBe("未分类");
    expect(groups).toHaveLength(3);
  });

  it("offers every category in use plus 未分类 to the picker", () => {
    const categories = categoriesOf([
      rule({ category: "住宿" }),
      rule({ category: "住宿" }),
      rule({ category: "餐饮" }),
    ]);
    expect(categories).toContain("住宿");
    expect(categories).toContain("餐饮");
    expect(categories).toContain("未分类");
    expect(categories).toHaveLength(3);
  });
});
