//! The rule set a new install starts with.
//!
//! Seeded once into the database on first run, then owned entirely by the
//! user - editing or deleting a default is permanent, and nothing here is
//! re-applied on upgrade. A rule set that silently reverted a user's edits
//! every time the app updated would be worse than shipping no defaults.
//!
//! ## How the priorities are laid out
//!
//! | 区间 | 用途 |
//! |------|------|
//! | 200+ | 用户接受 AI 建议后生成的规则（见 `proposed_rule`） |
//! | 150  | 按税收分类简称匹配 —— 税局自己的分类，最可靠 |
//! | 100  | 按票种匹配 —— 火车票、行程单这类本身就说明了用途 |
//! | 50   | 按销售方名称关键词兜底 —— 最容易误伤，排最后 |
//!
//! The ordering matters for the "为什么是这个类别" note more than for the
//! outcome: several rules often agree, and the user should be shown the most
//! specific reason rather than whichever one happened to be checked first.

use super::{Condition, MatchField, Rule};

/// `(类别, 优先级, 匹配字段, 关键词...)`
type Seed = (&'static str, i32, MatchField, &'static [&'static str]);

const SEEDS: &[Seed] = &[
    // --- 税收分类简称：税局自己的分类，最可靠 ---------------------------
    ("住宿", 150, MatchField::TaxCategory, &["住宿服务"]),
    ("餐饮", 150, MatchField::TaxCategory, &["餐饮服务"]),
    (
        "市内交通",
        150,
        MatchField::TaxCategory,
        &["旅客运输服务", "运输服务"],
    ),
    (
        "办公用品",
        150,
        MatchField::TaxCategory,
        &["办公用品", "文具", "计算机", "耗材"],
    ),
    (
        "通讯费",
        150,
        MatchField::TaxCategory,
        &["电信服务", "信息服务"],
    ),
    (
        "软件与服务",
        150,
        MatchField::TaxCategory,
        &["信息技术服务", "软件", "技术服务", "咨询服务"],
    ),
    (
        "快递邮寄",
        150,
        MatchField::TaxCategory,
        &["邮政服务", "收派服务", "物流辅助服务"],
    ),
    (
        "会议费",
        150,
        MatchField::TaxCategory,
        &["会议", "会展服务"],
    ),
    (
        "油费",
        150,
        MatchField::TaxCategory,
        &["成品油", "汽油", "柴油"],
    ),
    ("过路过桥费", 150, MatchField::TaxCategory, &["通行费"]),
    (
        "培训费",
        150,
        MatchField::TaxCategory,
        &["培训服务", "教育服务"],
    ),
    (
        "广告宣传",
        150,
        MatchField::TaxCategory,
        &["广告服务", "设计服务", "印刷"],
    ),
    (
        "房租物业",
        150,
        MatchField::TaxCategory,
        &["经营租赁", "不动产租赁", "物业管理"],
    ),
    // --- 票种：这些票据本身就说明了用途 ---------------------------------
    ("长途交通", 100, MatchField::Kind, &["铁路电子客票"]),
    (
        "长途交通",
        100,
        MatchField::Kind,
        &["航空运输电子客票行程单"],
    ),
    ("市内交通", 100, MatchField::Kind, &["出租车票"]),
    ("过路过桥费", 100, MatchField::Kind, &["过路过桥费发票"]),
    // --- 项目名称：分类简称不够细时的补充 -------------------------------
    (
        "长途交通",
        120,
        MatchField::ItemName,
        &["高铁", "动车", "火车票", "机票", "客票"],
    ),
    (
        "市内交通",
        120,
        MatchField::ItemName,
        &["网约车", "打车", "代驾", "停车"],
    ),
    // --- 销售方名称：最后的兜底，最容易误伤 -----------------------------
    (
        "市内交通",
        50,
        MatchField::SellerName,
        &["滴滴", "出行", "网约车", "出租汽车"],
    ),
    (
        "长途交通",
        50,
        MatchField::SellerName,
        &["航空", "铁路", "12306", "航空票务"],
    ),
    (
        "住宿",
        50,
        MatchField::SellerName,
        &["酒店", "宾馆", "旅馆", "民宿", "住宿"],
    ),
    (
        "餐饮",
        50,
        MatchField::SellerName,
        &["餐饮", "饭店", "餐厅", "食品", "茶饮", "咖啡"],
    ),
    (
        "软件与服务",
        50,
        MatchField::SellerName,
        &["科技", "网络", "信息技术", "云计算"],
    ),
    (
        "快递邮寄",
        50,
        MatchField::SellerName,
        &["顺丰", "快递", "速运", "物流", "邮政"],
    ),
    (
        "油费",
        50,
        MatchField::SellerName,
        &["石油", "石化", "加油"],
    ),
];

/// The categories the app knows about, in the order the reimbursement sheet
/// should总计 them. Derived from the seeds so the two can never drift, with
/// 未分类 pinned last.
pub fn categories() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (category, ..) in SEEDS {
        if !out.iter().any(|c| c == category) {
            out.push((*category).to_string());
        }
    }
    out.push(super::UNCATEGORISED.to_string());
    out
}

/// The seed rule set. Ids are assigned by the database on insert.
pub fn rules() -> Vec<Rule> {
    SEEDS
        .iter()
        .map(|(category, priority, field, keywords)| Rule {
            id: None,
            name: format!("{}：{}", field.label(), keywords.join("、")),
            category: (*category).to_string(),
            priority: *priority,
            enabled: true,
            conditions: vec![Condition {
                field: *field,
                keywords: keywords.iter().map(|k| (*k).to_string()).collect(),
            }],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{classify, Outcome};
    use crate::model::{Field, FieldSource, Invoice, InvoiceItem, InvoiceKind};

    fn with_item(seller: &str, item: &str) -> Invoice {
        Invoice {
            seller_name: Field::new(seller.to_string(), FieldSource::Xml),
            items: vec![InvoiceItem {
                name: item.to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn category_of(invoice: &Invoice) -> String {
        match classify(invoice, &rules()) {
            Outcome::Matched { category, .. } => category,
            Outcome::Unmatched => "未匹配".to_string(),
        }
    }

    #[test]
    fn the_common_expense_types_are_covered_out_of_the_box() {
        let cases = [
            ("杭州云栖酒店管理有限公司", "*住宿服务*住宿费", "住宿"),
            ("上海某某餐饮管理有限公司", "*餐饮服务*餐费", "餐饮"),
            ("滴滴出行科技有限公司", "*运输服务*客运服务费", "市内交通"),
            ("某某文具有限公司", "*办公用品*A4纸", "办公用品"),
            ("中国移动通信集团", "*电信服务*套餐费", "通讯费"),
            ("某某科技有限公司", "*信息技术服务*技术服务费", "软件与服务"),
            ("顺丰速运有限公司", "*收派服务*快递费", "快递邮寄"),
            ("中国石化销售股份有限公司", "*成品油*92号汽油", "油费"),
        ];
        for (seller, item, expected) in cases {
            assert_eq!(category_of(&with_item(seller, item)), expected, "{item}");
        }
    }

    /// Transport receipts carry no line items at all - the 票种 rules are the
    /// only thing that can categorise them.
    #[test]
    fn transport_receipts_categorise_on_kind_alone() {
        let train = Invoice {
            kind: Some(InvoiceKind::Train),
            ..Default::default()
        };
        assert_eq!(category_of(&train), "长途交通");

        let taxi = Invoice {
            kind: Some(InvoiceKind::Taxi),
            ..Default::default()
        };
        assert_eq!(category_of(&taxi), "市内交通");
    }

    /// The tax-category rule must outrank the seller-name fallback, so the
    /// reason shown to the user is the specific one.
    #[test]
    fn the_specific_reason_is_the_one_credited() {
        let inv = with_item("某某大酒店有限公司", "*餐饮服务*餐费");
        match classify(&inv, &rules()) {
            Outcome::Matched { category, rule } => {
                assert_eq!(category, "餐饮", "酒店里的餐费仍是餐饮");
                assert!(
                    rule.contains("税收分类简称"),
                    "应归功于分类简称规则: {rule}"
                );
            }
            other => panic!("未匹配: {other:?}"),
        }
    }

    #[test]
    fn an_unknown_invoice_stays_unmatched_rather_than_guessing() {
        let inv = with_item("某某有限公司", "*不知道什么*莫名其妙的东西");
        assert_eq!(classify(&inv, &rules()), Outcome::Unmatched);
    }

    #[test]
    fn categories_are_unique_and_end_with_uncategorised() {
        let cats = categories();
        let mut sorted = cats.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), cats.len(), "类别有重复");
        assert_eq!(cats.last().unwrap(), crate::classify::UNCATEGORISED);
    }

    #[test]
    fn every_seed_rule_is_well_formed() {
        for rule in rules() {
            assert!(!rule.conditions.is_empty(), "{} 无条件", rule.name);
            assert!(
                rule.conditions[0].keywords.iter().all(|k| !k.is_empty()),
                "{} 含空关键词",
                rule.name
            );
        }
    }
}
