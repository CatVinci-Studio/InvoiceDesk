//! Assigning a 报销类别 to an invoice.
//!
//! ## Rules first, model last
//!
//! Categorisation looks like a job for a language model, and it is a bad one.
//! The same twenty vendors turn up month after month in any real expense
//! stream, and a rule that matched 滴滴出行 to 市内交通 in March is still
//! right in November - for free, offline, instantly, and identically every
//! time. A model asked the same question twice can answer differently, which
//! in a spreadsheet that someone reconciles against last month's is a defect,
//! not a nuance.
//!
//! So: rules decide, and the model only sees what no rule matched. Its answer
//! is then offered back to the user **as a proposed rule**, which is the part
//! that compounds - every model call the user accepts is one that never has
//! to happen again.
//!
//! ## Why 商品税收分类简称 is the strongest signal
//!
//! Every VAT invoice line item is printed as `*类别简称*具体名称`, and the
//! starred prefix is the tax authority's own classification - assigned by the
//! issuer from a fixed national list, not free text. `*住宿服务*` means
//! accommodation on every invoice in China. Matching that beats matching the
//! seller's name, which varies endlessly ("XX酒店管理有限公司",
//! "XX商务宾馆", "XX酒店（上海）有限公司"), so it is checked first.

pub mod defaults;

use crate::model::{Invoice, InvoiceKind};
use serde::{Deserialize, Serialize};

/// Which part of an invoice a condition looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchField {
    /// The `*...*` prefix of any line item - the tax authority's own code.
    TaxCategory,
    /// The full line item text, for when the starred prefix is too broad.
    ItemName,
    SellerName,
    Remark,
    /// 票种, matched by its display label.
    Kind,
}

impl MatchField {
    pub fn label(self) -> &'static str {
        match self {
            MatchField::TaxCategory => "税收分类简称",
            MatchField::ItemName => "项目名称",
            MatchField::SellerName => "销售方名称",
            MatchField::Remark => "备注",
            MatchField::Kind => "票种",
        }
    }
}

/// One condition: does `field` contain any of `keywords`?
///
/// "Any of" rather than "all of" because keywords are synonyms for the same
/// concept (酒店 / 宾馆 / 住宿), which is how a user thinks about them. Needing
/// several *different* things to be true is expressed by adding conditions,
/// which is the other axis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    pub field: MatchField,
    pub keywords: Vec<String>,
}

impl Condition {
    fn matches(&self, invoice: &Invoice) -> bool {
        let haystacks: Vec<String> = match self.field {
            MatchField::TaxCategory => invoice
                .items
                .iter()
                .filter_map(|i| i.tax_category())
                .map(str::to_string)
                .collect(),
            MatchField::ItemName => invoice.items.iter().map(|i| i.name.clone()).collect(),
            MatchField::SellerName => invoice
                .seller_name
                .as_ref()
                .map(|s| vec![s.clone()])
                .unwrap_or_default(),
            MatchField::Remark => invoice
                .remark
                .as_ref()
                .map(|s| vec![s.clone()])
                .unwrap_or_default(),
            MatchField::Kind => invoice
                .kind
                .map(|k| vec![k.label().to_string()])
                .unwrap_or_default(),
        };

        haystacks.iter().any(|haystack| {
            self.keywords
                .iter()
                .any(|k| !k.is_empty() && haystack.contains(k.as_str()))
        })
    }
}

/// A categorisation rule. All conditions must hold (AND); several rules can
/// target the same category to express alternatives (OR).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: Option<i64>,
    /// Shown in the "why this category?" note, so it should read as a reason.
    pub name: String,
    pub category: String,
    /// Higher wins. Specific rules (`*住宿服务*`) sit above catch-alls
    /// (seller name contains 酒店) so the specific one is what gets credited.
    pub priority: i32,
    pub enabled: bool,
    pub conditions: Vec<Condition>,
}

impl Rule {
    fn matches(&self, invoice: &Invoice) -> bool {
        // A rule with no conditions would match everything - almost certainly
        // a half-finished edit in the rule editor rather than an intent to
        // recategorise the entire ledger.
        self.enabled
            && !self.conditions.is_empty()
            && self.conditions.iter().all(|c| c.matches(invoice))
    }
}

/// What the classifier concluded.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Matched {
        category: String,
        rule: String,
    },
    /// No rule matched. The caller may ask a model - see
    /// [`crate::ai::vision`] - or leave it for the user.
    Unmatched,
}

/// Applies the rule set, highest priority first.
///
/// Ties break on rule order, which is stable because the caller sorts by id -
/// so the same invoice and the same rules always give the same answer, which
/// is the whole reason rules are preferred over a model.
pub fn classify(invoice: &Invoice, rules: &[Rule]) -> Outcome {
    let mut candidates: Vec<&Rule> = rules.iter().filter(|r| r.matches(invoice)).collect();
    candidates.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.id.unwrap_or(i64::MAX).cmp(&b.id.unwrap_or(i64::MAX)))
    });

    match candidates.first() {
        Some(rule) => Outcome::Matched {
            category: rule.category.clone(),
            rule: rule.name.clone(),
        },
        None => Outcome::Unmatched,
    }
}

/// The category to use for anything a rule never claimed and the user never
/// set. Named rather than empty so it groups and totals like any other.
pub const UNCATEGORISED: &str = "未分类";

/// A category suggested by a model, together with the rule that would make
/// the suggestion permanent. Presenting both is what turns a one-off answer
/// into something that stops costing an API call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub category: String,
    /// Why the model chose it, shown next to the accept button.
    pub reason: String,
    pub proposed_rule: Rule,
}

/// Builds the rule a model's answer implies: match the invoice's own tax
/// category if it has one, otherwise its seller.
///
/// Tax category is preferred because it generalises to every other vendor in
/// the same trade, while a seller-name rule only ever helps with that one
/// company.
pub fn proposed_rule(invoice: &Invoice, category: &str) -> Option<Rule> {
    let (field, keyword) = invoice
        .items
        .iter()
        .find_map(|i| i.tax_category())
        .map(|c| (MatchField::TaxCategory, c.to_string()))
        .or_else(|| {
            invoice
                .seller_name
                .as_ref()
                .map(|s| (MatchField::SellerName, s.clone()))
        })?;

    Some(Rule {
        id: None,
        name: format!("{}：{}", field.label(), keyword),
        category: category.to_string(),
        // Above the built-ins, because a rule the user accepted for their own
        // expense stream is more specific to them than any default.
        priority: 200,
        enabled: true,
        conditions: vec![Condition {
            field,
            keywords: vec![keyword],
        }],
    })
}

/// The 票种 a category is *usually* carried on, used only to seed the rule
/// editor's dropdown. Not a constraint.
pub fn kinds_for_hint(category: &str) -> Vec<InvoiceKind> {
    match category {
        "长途交通" => vec![InvoiceKind::Train, InvoiceKind::AirItinerary],
        "市内交通" => vec![InvoiceKind::Taxi],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Field, FieldSource, InvoiceItem};

    fn invoice(seller: &str, item: &str) -> Invoice {
        Invoice {
            seller_name: Field::new(seller.to_string(), FieldSource::Xml),
            items: vec![InvoiceItem {
                name: item.to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn rule(name: &str, category: &str, priority: i32, field: MatchField, kw: &[&str]) -> Rule {
        Rule {
            id: Some(1),
            name: name.to_string(),
            category: category.to_string(),
            priority,
            enabled: true,
            conditions: vec![Condition {
                field,
                keywords: kw.iter().map(|s| s.to_string()).collect(),
            }],
        }
    }

    #[test]
    fn the_tax_category_prefix_drives_the_match() {
        let inv = invoice("杭州云栖酒店管理有限公司", "*住宿服务*住宿费");
        let rules = vec![rule(
            "住宿",
            "住宿",
            100,
            MatchField::TaxCategory,
            &["住宿服务"],
        )];
        assert_eq!(
            classify(&inv, &rules),
            Outcome::Matched {
                category: "住宿".to_string(),
                rule: "住宿".to_string()
            }
        );
    }

    /// The specific rule must be the one credited, so "why this category?"
    /// names something the user can act on.
    #[test]
    fn higher_priority_wins() {
        let inv = invoice("某某酒店有限公司", "*餐饮服务*餐费");
        let rules = vec![
            rule(
                "销售方含酒店",
                "住宿",
                50,
                MatchField::SellerName,
                &["酒店"],
            ),
            rule(
                "餐饮服务",
                "餐饮",
                150,
                MatchField::TaxCategory,
                &["餐饮服务"],
            ),
        ];
        match classify(&inv, &rules) {
            Outcome::Matched { category, rule } => {
                assert_eq!(category, "餐饮");
                assert_eq!(rule, "餐饮服务");
            }
            other => panic!("未匹配: {other:?}"),
        }
    }

    #[test]
    fn all_conditions_must_hold() {
        let inv = invoice("某公司", "*住宿服务*住宿费");
        let both = Rule {
            conditions: vec![
                Condition {
                    field: MatchField::TaxCategory,
                    keywords: vec!["住宿服务".into()],
                },
                Condition {
                    field: MatchField::SellerName,
                    keywords: vec!["航空".into()],
                },
            ],
            ..rule("组合", "差旅", 100, MatchField::TaxCategory, &["住宿服务"])
        };
        assert_eq!(classify(&inv, &[both]), Outcome::Unmatched);
    }

    #[test]
    fn disabled_and_empty_rules_never_match() {
        let inv = invoice("某公司", "*住宿服务*住宿费");
        let mut disabled = rule("住宿", "住宿", 100, MatchField::TaxCategory, &["住宿服务"]);
        disabled.enabled = false;
        assert_eq!(classify(&inv, &[disabled]), Outcome::Unmatched);

        // An empty condition list would otherwise match every invoice ever.
        let empty = Rule {
            conditions: vec![],
            ..rule("空", "住宿", 999, MatchField::TaxCategory, &[])
        };
        assert_eq!(classify(&inv, &[empty]), Outcome::Unmatched);
    }

    #[test]
    fn classification_is_deterministic_across_runs() {
        let inv = invoice("某公司", "*住宿服务*住宿费");
        let rules = vec![
            rule("A", "住宿", 100, MatchField::TaxCategory, &["住宿服务"]),
            Rule {
                id: Some(2),
                ..rule("B", "差旅", 100, MatchField::TaxCategory, &["住宿服务"])
            },
        ];
        // Equal priority, so the lower id decides - every time.
        for _ in 0..5 {
            assert_eq!(
                classify(&inv, &rules),
                Outcome::Matched {
                    category: "住宿".to_string(),
                    rule: "A".to_string()
                }
            );
        }
    }

    /// A proposed rule should generalise beyond the one vendor that
    /// triggered it, which means preferring the tax category.
    #[test]
    fn proposed_rules_prefer_the_tax_category_over_the_seller() {
        let inv = invoice("杭州云栖酒店管理有限公司", "*住宿服务*住宿费");
        let proposed = proposed_rule(&inv, "住宿").unwrap();
        assert_eq!(proposed.conditions[0].field, MatchField::TaxCategory);
        assert_eq!(proposed.conditions[0].keywords, vec!["住宿服务"]);
    }

    #[test]
    fn proposed_rules_fall_back_to_the_seller_when_there_are_no_line_items() {
        let inv = Invoice {
            seller_name: Field::new("滴滴出行科技有限公司".to_string(), FieldSource::Vision),
            ..Default::default()
        };
        let proposed = proposed_rule(&inv, "市内交通").unwrap();
        assert_eq!(proposed.conditions[0].field, MatchField::SellerName);
    }

    #[test]
    fn an_invoice_with_nothing_to_match_on_proposes_nothing() {
        assert!(proposed_rule(&Invoice::default(), "其他").is_none());
    }
}
