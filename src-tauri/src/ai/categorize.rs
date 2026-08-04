//! Asking a model which 报销类别 an invoice belongs to, when no rule matched.
//!
//! Runs on TEXT, not on the invoice image: the seller's name and the line
//! items are the whole question, and they are already extracted. That keeps
//! this switch meaningfully separate from the vision one - a user can allow
//! a company name to leave the machine while refusing to upload a picture of
//! the invoice itself.
//!
//! The answer is never applied silently. It comes back as a
//! [`Suggestion`](crate::classify::Suggestion) carrying a **proposed rule**,
//! and accepting that rule is what makes the next invoice from the same
//! vendor free. A tool that quietly guessed every month would cost the same
//! every month and teach the user nothing about why.

use crate::classify::{proposed_rule, Suggestion};
use crate::model::Invoice;
use aicompat::agent::{AgentTurn, StepResult};
use aicompat::providers::custom;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Answer {
    #[serde(rename = "类别")]
    category: String,
    #[serde(rename = "理由", default)]
    reason: String,
}

fn system_prompt(categories: &[String]) -> String {
    format!(
        "你是企业报销分类助手。用户会给你一张发票的关键信息，你要从下面这个**固定列表**里选出最合适的费用类别。\n\n\
         可选类别：{}\n\n\
         规则：\n\
         1. 只能从上面的列表里选，不要发明新类别。\n\
         2. 实在无法判断时选「未分类」，不要勉强归类。\n\
         3. 只输出 JSON，不要解释：{{\"类别\": \"...\", \"理由\": \"一句话，20 字以内\"}}",
        categories.join("、")
    )
}

/// The facts the model is given. Deliberately narrow: the seller, the goods,
/// and the 票种. No amounts, no tax IDs, no buyer - none of it helps decide a
/// category, and not sending it is the cheapest privacy measure there is.
fn describe(invoice: &Invoice) -> String {
    let mut lines = Vec::new();
    if let Some(kind) = invoice.kind {
        lines.push(format!("票种：{}", kind.label()));
    }
    if let Some(seller) = invoice.seller_name.as_ref() {
        lines.push(format!("销售方：{seller}"));
    }
    for item in invoice.items.iter().take(5) {
        lines.push(format!("项目：{}", item.name));
    }
    if let Some(remark) = invoice.remark.as_ref() {
        lines.push(format!("备注：{remark}"));
    }
    lines.join("\n")
}

fn extract_json(reply: &str) -> Option<&str> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    (end > start).then(|| &reply[start..=end])
}

/// Asks for a category. `categories` is the list the user's own rules
/// produce, so the model can only ever answer with something the rest of the
/// app already understands.
pub async fn suggest(
    endpoint: &super::route::Endpoint,
    invoice: &Invoice,
    categories: &[String],
) -> Result<Suggestion, String> {
    let description = describe(invoice);
    if description.trim().is_empty() {
        return Err("这张发票没有可用于判断类别的信息。".to_string());
    }

    let step = custom::agent_step(
        &endpoint.base_url,
        endpoint.api_key.as_deref(),
        &endpoint.model,
        &system_prompt(categories),
        &[AgentTurn::User {
            text: description,
            images: Vec::new(),
        }],
        &[],
        None,
    )
    .await?;

    let reply = match step {
        StepResult::Done(text) => text,
        StepResult::ToolCalls(_) => return Err("模型返回了意外的响应格式".to_string()),
    };

    parse_suggestion(&reply, invoice, categories)
}

/// The pure half, so the answer handling is testable without a network call.
fn parse_suggestion(
    reply: &str,
    invoice: &Invoice,
    categories: &[String],
) -> Result<Suggestion, String> {
    let json = extract_json(reply).ok_or_else(|| "模型没有返回 JSON".to_string())?;
    let answer: Answer =
        serde_json::from_str(json).map_err(|e| format!("模型返回的 JSON 无法解析：{e}"))?;

    let category = answer.category.trim().to_string();
    // A category outside the list would create a phantom group in the
    // summary that no rule can ever reproduce - so an off-list answer is
    // rejected rather than accepted and quietly added.
    if !categories.iter().any(|c| c == &category) {
        return Err(format!("模型给出了列表之外的类别「{category}」，已忽略。"));
    }

    let proposed = proposed_rule(invoice, &category)
        .ok_or_else(|| "这张发票没有可以写成规则的特征。".to_string())?;

    Ok(Suggestion {
        category,
        reason: answer.reason.trim().to_string(),
        proposed_rule: proposed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Field, FieldSource, InvoiceItem, InvoiceKind};

    fn categories() -> Vec<String> {
        vec![
            "住宿".into(),
            "餐饮".into(),
            "市内交通".into(),
            "未分类".into(),
        ]
    }

    fn invoice() -> Invoice {
        Invoice {
            kind: Some(InvoiceKind::DigitalGeneral),
            seller_name: Field::new("杭州云栖酒店管理有限公司".into(), FieldSource::Xml),
            items: vec![InvoiceItem {
                name: "*住宿服务*住宿费".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn a_valid_answer_becomes_a_suggestion_with_a_rule() {
        let suggestion = parse_suggestion(
            r#"{"类别": "住宿", "理由": "酒店住宿费"}"#,
            &invoice(),
            &categories(),
        )
        .unwrap();

        assert_eq!(suggestion.category, "住宿");
        assert_eq!(suggestion.reason, "酒店住宿费");
        // The proposed rule generalises via the tax category, so every other
        // hotel is covered too.
        assert_eq!(suggestion.proposed_rule.category, "住宿");
        assert_eq!(
            suggestion.proposed_rule.conditions[0].keywords,
            vec!["住宿服务"]
        );
    }

    /// An off-list answer would create a group the summary shows and no rule
    /// can reproduce.
    #[test]
    fn an_invented_category_is_rejected() {
        let error = parse_suggestion(
            r#"{"类别": "差旅住宿费用", "理由": "..."}"#,
            &invoice(),
            &categories(),
        )
        .unwrap_err();
        assert!(error.contains("列表之外"), "{error}");
    }

    #[test]
    fn json_is_recovered_from_a_fenced_reply() {
        let suggestion = parse_suggestion(
            "```json\n{\"类别\": \"住宿\", \"理由\": \"酒店\"}\n```",
            &invoice(),
            &categories(),
        )
        .unwrap();
        assert_eq!(suggestion.category, "住宿");
    }

    #[test]
    fn a_non_json_reply_is_an_error() {
        assert!(parse_suggestion("我不确定这是什么发票。", &invoice(), &categories()).is_err());
    }

    /// The prompt must not carry money or tax IDs - they do not help decide a
    /// category, and not sending them is free.
    #[test]
    fn the_prompt_carries_only_what_the_question_needs() {
        let mut inv = invoice();
        inv.total = Field::new(crate::model::Money(106_000), FieldSource::Xml);
        inv.buyer_tax_id = Field::new("91310115MA1K3XYZ2B".into(), FieldSource::Xml);
        inv.buyer_name = Field::new("猫芬奇工作室".into(), FieldSource::Xml);

        let description = describe(&inv);
        assert!(description.contains("杭州云栖酒店管理有限公司"));
        assert!(description.contains("*住宿服务*住宿费"));
        assert!(!description.contains("1060"), "不应发送金额");
        assert!(!description.contains("91310115MA1K3XYZ2B"), "不应发送税号");
        assert!(!description.contains("猫芬奇工作室"), "不应发送购买方");
    }

    #[test]
    fn an_invoice_with_nothing_to_describe_is_not_sent() {
        let empty = Invoice::default();
        assert!(describe(&empty).trim().is_empty());
    }

    /// The list in the prompt is the user's own category list, so the model
    /// cannot answer with anything the app does not already handle.
    #[test]
    fn the_prompt_lists_the_users_own_categories() {
        let prompt = system_prompt(&categories());
        for category in categories() {
            assert!(prompt.contains(&category), "缺少类别 {category}");
        }
    }
}
