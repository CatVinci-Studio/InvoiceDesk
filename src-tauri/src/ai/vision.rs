//! The last extraction layer: asking a vision model to read a photo.
//!
//! Reached only when the offline layers came up short - see
//! [`crate::extract`]. That ordering is the whole design: a QR code or a text
//! layer is exact and free, and this is neither, so it runs last and its
//! results are marked [`FieldSource::Vision`] so every value it produces
//! stays flagged for review.
//!
//! ## Three rules the prompt enforces
//!
//! 1. **Return `null` rather than guessing.** A model that invents a
//!    plausible 发票号码 is worse than one that admits it cannot read the
//!    photo: the invented number passes every shape check, defeats duplicate
//!    detection, and nobody ever looks at it again. The prompt says this
//!    twice because it is the single most important instruction in it.
//! 2. **Copy digits exactly, do not compute.** Asking a language model to
//!    add 金额 and 税额 invites arithmetic that looks right. The app derives
//!    those relationships itself, in integers, in
//!    [`crate::parse::validate`].
//! 3. **JSON only.** Chat models like to explain themselves; the parser
//!    tolerates fences and prose around the object, but the prompt asks for
//!    a bare object so the common case is cheap.

use crate::model::{Field, FieldSource, Invoice, InvoiceItem, InvoiceKind, Money};
use aicompat::agent::{AgentTurn, ImageAttachment, StepResult};
use aicompat::providers::custom;
use image::DynamicImage;
use serde::Deserialize;

/// Longest edge, in pixels, that an invoice photo is scaled down to before
/// being sent.
///
/// Chosen against the smallest thing that must stay legible - the 8pt digits
/// of a 发票号码. A 1600px-wide frame leaves those around 12px tall, which is
/// comfortable; going higher multiplies upload size and latency for detail
/// no model needs, and going much lower starts losing digits.
const MAX_EDGE: u32 = 1600;

/// JPEG quality for the upload. 85 is above the point where 宋体 strokes
/// start to smear and well below the point where the file stops shrinking.
const JPEG_QUALITY: u8 = 85;

const SYSTEM_PROMPT: &str = "\
你是中国增值税发票识别助手。用户会给你一张发票的图片，你只需要把图片上**已经印着的**字段照抄出来，输出一个 JSON 对象。

严格遵守：
1. 看不清、被遮挡、或者图片上根本没有的字段，一律填 null。**绝对不要猜测、不要根据常识补全、不要编造。**填 null 是完全可以接受的正确答案。
2. 数字逐位照抄。发票号码可能是 8 位或 20 位，一位都不能多、不能少、不能改。
3. 不要做任何计算。金额、税额、价税合计各自照抄票面上印的数字，即使它们看起来对不上也照抄。
4. 金额一律用阿拉伯数字字符串，如 \"1060.00\"，不要带 ¥ 或千分位逗号。
5. 日期统一写成 YYYY-MM-DD。
6. 只输出 JSON 对象本身，不要解释、不要 Markdown 代码块。

JSON 结构：
{
  \"票种\": \"增值税专用发票|增值税普通发票|增值税电子普通发票|电子发票(普通发票)|电子发票(增值税专用发票)|铁路电子客票|航空运输电子客票行程单|出租车票|定额发票|通行费发票|其他\" 或 null,
  \"发票号码\": \"...\" 或 null,
  \"发票代码\": \"...\" 或 null,
  \"开票日期\": \"YYYY-MM-DD\" 或 null,
  \"购买方名称\": \"...\" 或 null,
  \"购买方纳税人识别号\": \"...\" 或 null,
  \"销售方名称\": \"...\" 或 null,
  \"销售方纳税人识别号\": \"...\" 或 null,
  \"金额\": \"不含税金额\" 或 null,
  \"税额\": \"...\" 或 null,
  \"价税合计\": \"...\" 或 null,
  \"备注\": \"...\" 或 null,
  \"明细\": [ { \"名称\": \"...\", \"金额\": \"...\", \"税率\": \"...\", \"税额\": \"...\" } ],
  \"看不清的字段\": [\"字段名\", ...]
}";

const USER_PROMPT: &str = "请识别这张发票，按要求输出 JSON。看不清的字段填 null。";

/// The model's answer, before it is folded into an [`Invoice`].
#[derive(Debug, Default, Deserialize)]
struct VisionReading {
    #[serde(rename = "票种")]
    kind: Option<String>,
    #[serde(rename = "发票号码")]
    number: Option<String>,
    #[serde(rename = "发票代码")]
    code: Option<String>,
    #[serde(rename = "开票日期")]
    issued_on: Option<String>,
    #[serde(rename = "购买方名称")]
    buyer_name: Option<String>,
    #[serde(rename = "购买方纳税人识别号")]
    buyer_tax_id: Option<String>,
    #[serde(rename = "销售方名称")]
    seller_name: Option<String>,
    #[serde(rename = "销售方纳税人识别号")]
    seller_tax_id: Option<String>,
    #[serde(rename = "金额")]
    amount_excl_tax: Option<String>,
    #[serde(rename = "税额")]
    tax: Option<String>,
    #[serde(rename = "价税合计")]
    total: Option<String>,
    #[serde(rename = "备注")]
    remark: Option<String>,
    #[serde(rename = "明细", default)]
    items: Vec<VisionItem>,
    /// Fields the model itself flagged as illegible. Used to push those
    /// fields' confidence down further - a model that volunteers doubt is
    /// giving away real information and it would be wasteful to discard it.
    #[serde(rename = "看不清的字段", default)]
    unsure: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct VisionItem {
    #[serde(rename = "名称")]
    name: Option<String>,
    #[serde(rename = "金额")]
    amount: Option<String>,
    #[serde(rename = "税率")]
    tax_rate: Option<String>,
    #[serde(rename = "税额")]
    tax: Option<String>,
}

/// Pulls the JSON object out of a chat reply.
///
/// Models wrap answers in ```json fences, prefix them with "好的，识别结果
/// 如下：", or both. Slicing from the first `{` to the last `}` handles every
/// variant seen without a per-provider special case.
fn extract_json(reply: &str) -> Option<&str> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    (end > start).then(|| &reply[start..=end])
}

/// Empty strings, "null", "无", "-" all mean "not present" - models use them
/// interchangeably despite the prompt asking for `null`.
fn meaningful(value: Option<String>) -> Option<String> {
    let text = value?.trim().to_string();
    if text.is_empty() || matches!(text.as_str(), "null" | "N/A" | "无" | "-" | "—" | "未知") {
        return None;
    }
    Some(text)
}

impl VisionReading {
    fn apply(self, invoice: &mut Invoice) {
        // `unsure` is taken out of `self` up front so the per-field closures
        // below can borrow it while the rest of `self` is moved field by
        // field into the invoice.
        let unsure = self.unsure;

        /// Confidence for one field: the source's baseline, halved when the
        /// model itself said it could not read that field.
        fn confidence(unsure: &[String], field: &str) -> f32 {
            let base = FieldSource::Vision.base_confidence();
            if unsure.iter().any(|f| f.contains(field)) {
                base * 0.5
            } else {
                base
            }
        }

        let text = |field: &str, value: Option<String>, slot: &mut Field<String>| {
            if let Some(value) = meaningful(value) {
                slot.merge_from(Field::with_confidence(
                    value,
                    FieldSource::Vision,
                    confidence(&unsure, field),
                ));
            }
        };

        text("发票号码", self.number, &mut invoice.number);
        text("发票代码", self.code, &mut invoice.code);
        text("开票日期", self.issued_on, &mut invoice.issued_on);
        text("购买方名称", self.buyer_name, &mut invoice.buyer_name);
        text("购买方", self.buyer_tax_id, &mut invoice.buyer_tax_id);
        text("销售方名称", self.seller_name, &mut invoice.seller_name);
        text("销售方", self.seller_tax_id, &mut invoice.seller_tax_id);
        text("备注", self.remark, &mut invoice.remark);

        let money = |field: &str, value: Option<String>, slot: &mut Field<Money>| {
            if let Some(parsed) = meaningful(value).as_deref().and_then(Money::parse) {
                slot.merge_from(Field::with_confidence(
                    parsed,
                    FieldSource::Vision,
                    confidence(&unsure, field),
                ));
            }
        };
        money("金额", self.amount_excl_tax, &mut invoice.amount_excl_tax);
        money("税额", self.tax, &mut invoice.tax);
        money("价税合计", self.total, &mut invoice.total);

        if invoice.kind.is_none() {
            invoice.kind = meaningful(self.kind)
                .as_deref()
                .and_then(InvoiceKind::from_title);
        }

        // Line items only fill a gap: a structured source that produced them
        // has better ones, and they are what drives categorisation.
        if invoice.items.is_empty() {
            invoice.items = self
                .items
                .into_iter()
                .filter_map(|item| {
                    let name = meaningful(item.name)?;
                    Some(InvoiceItem {
                        name,
                        amount: meaningful(item.amount).as_deref().and_then(Money::parse),
                        tax_rate: meaningful(item.tax_rate),
                        tax: meaningful(item.tax).as_deref().and_then(Money::parse),
                        ..Default::default()
                    })
                })
                .collect();
        }
    }
}

/// Scales an image down and encodes it as JPEG for upload.
pub fn encode_for_upload(image: &DynamicImage) -> Result<ImageAttachment, String> {
    use base64::Engine;

    let (width, height) = (image.width(), image.height());
    let scaled = if width.max(height) > MAX_EDGE {
        // `resize` preserves aspect ratio and fits inside the box; Lanczos3
        // keeps thin strokes intact where a cheaper filter blurs them into
        // the background, which is the difference between a readable 8 and
        // an unreadable one.
        image.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Lanczos3)
    } else {
        image.clone()
    };

    let mut buffer = std::io::Cursor::new(Vec::new());
    // JPEG has no alpha channel; an RGBA source would fail to encode.
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, JPEG_QUALITY)
        .encode_image(&scaled.to_rgb8())
        .map_err(|e| format!("图片编码失败：{e}"))?;

    Ok(ImageAttachment {
        name: "invoice.jpg".to_string(),
        mime: "image/jpeg".to_string(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(buffer.into_inner()),
    })
}

/// Sends one invoice image and folds the answer into `invoice`.
///
/// Returns the raw reply alongside, so the detail pane can show what the
/// model actually said when a user disputes a field.
pub async fn read_invoice(
    endpoint: &super::route::Endpoint,
    image: &DynamicImage,
    invoice: &mut Invoice,
) -> Result<String, String> {
    let attachment = encode_for_upload(image)?;
    let turn = AgentTurn::User {
        text: USER_PROMPT.to_string(),
        images: vec![attachment],
    };

    let step = custom::agent_step(
        &endpoint.base_url,
        endpoint.api_key.as_deref(),
        &endpoint.model,
        SYSTEM_PROMPT,
        &[turn],
        // No tools and no web search: this is one question about one picture.
        &[],
        None,
    )
    .await?;

    let reply = match step {
        StepResult::Done(text) => text,
        // A model that answered with a tool call when given no tools has
        // misunderstood the request entirely; there is nothing to salvage.
        StepResult::ToolCalls(_) => return Err("模型返回了意外的响应格式".to_string()),
    };

    apply_reply(&reply, invoice)?;
    Ok(reply)
}

/// The pure half of [`read_invoice`], so the parsing can be tested without a
/// network round trip.
fn apply_reply(reply: &str, invoice: &mut Invoice) -> Result<(), String> {
    let json = extract_json(reply).ok_or_else(|| "模型没有返回 JSON".to_string())?;
    let reading: VisionReading =
        serde_json::from_str(json).map_err(|e| format!("模型返回的 JSON 无法解析：{e}"))?;
    reading.apply(invoice);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_REPLY: &str = r#"{
        "票种": "电子发票(普通发票)",
        "发票号码": "24312000000012345678",
        "发票代码": null,
        "开票日期": "2024-03-01",
        "购买方名称": "猫芬奇工作室",
        "购买方纳税人识别号": "91310115MA1K3XYZ2B",
        "销售方名称": "杭州云栖酒店管理有限公司",
        "销售方纳税人识别号": "91330106MA2GX8QP1A",
        "金额": "1000.00",
        "税额": "60.00",
        "价税合计": "1060.00",
        "备注": null,
        "明细": [{"名称": "*住宿服务*住宿费", "金额": "1000.00", "税率": "6%", "税额": "60.00"}],
        "看不清的字段": []
    }"#;

    #[test]
    fn reads_a_well_formed_reply() {
        let mut invoice = Invoice::default();
        apply_reply(GOOD_REPLY, &mut invoice).unwrap();

        assert_eq!(
            invoice.number.value.as_deref(),
            Some("24312000000012345678")
        );
        assert_eq!(invoice.total.value, Some(Money(106_000)));
        assert_eq!(invoice.kind, Some(InvoiceKind::DigitalGeneral));
        assert_eq!(invoice.items.len(), 1);
        assert_eq!(invoice.items[0].tax_category(), Some("住宿服务"));
    }

    /// Everything a model read stays flagged, however confident it sounded.
    #[test]
    fn every_vision_field_needs_review() {
        let mut invoice = Invoice::default();
        apply_reply(GOOD_REPLY, &mut invoice).unwrap();
        assert_eq!(invoice.total.source, FieldSource::Vision);
        assert!(invoice.total.needs_review());
        assert!(invoice.number.needs_review());
    }

    /// A QR code or an XML attachment beats a model reading, no matter which
    /// ran last.
    #[test]
    fn a_better_source_is_never_overwritten() {
        let mut invoice = Invoice {
            number: Field::new("99999999999999999999".to_string(), FieldSource::QrCode),
            total: Field::new(Money(99_999), FieldSource::Xml),
            ..Default::default()
        };
        apply_reply(GOOD_REPLY, &mut invoice).unwrap();
        assert_eq!(
            invoice.number.value.as_deref(),
            Some("99999999999999999999"),
            "二维码读到的号码不该被模型覆盖"
        );
        assert_eq!(invoice.total.value, Some(Money(99_999)));
        // But it does fill what the better source left empty.
        assert_eq!(
            invoice.seller_name.value.as_deref(),
            Some("杭州云栖酒店管理有限公司")
        );
    }

    /// A model that admits doubt is telling us something worth keeping.
    #[test]
    fn self_reported_doubt_lowers_confidence_further() {
        let reply = r#"{"发票号码": "24312000000012345678", "价税合计": "1060.00",
                        "看不清的字段": ["发票号码"]}"#;
        let mut invoice = Invoice::default();
        apply_reply(reply, &mut invoice).unwrap();
        assert!(invoice.number.confidence < invoice.total.confidence);
    }

    #[test]
    fn json_is_recovered_from_fences_and_chatter() {
        let wrapped = format!("好的，识别结果如下：\n```json\n{GOOD_REPLY}\n```\n希望有帮助！");
        let mut invoice = Invoice::default();
        apply_reply(&wrapped, &mut invoice).unwrap();
        assert_eq!(invoice.total.value, Some(Money(106_000)));
    }

    /// The placeholders models reach for instead of `null`.
    #[test]
    fn placeholder_strings_are_treated_as_missing() {
        let reply = r#"{"发票号码": "", "销售方名称": "无", "备注": "N/A",
                        "购买方名称": "  ", "价税合计": "1060.00"}"#;
        let mut invoice = Invoice::default();
        apply_reply(reply, &mut invoice).unwrap();
        assert_eq!(invoice.number.value, None);
        assert_eq!(invoice.seller_name.value, None);
        assert_eq!(invoice.remark.value, None);
        assert_eq!(invoice.buyer_name.value, None);
        assert_eq!(invoice.total.value, Some(Money(106_000)));
    }

    #[test]
    fn a_reply_with_no_json_is_an_error_not_an_empty_invoice() {
        let mut invoice = Invoice::default();
        assert!(apply_reply("抱歉，图片太模糊了，我看不清。", &mut invoice).is_err());
    }

    #[test]
    fn digital_special_is_not_read_as_paper_special() {
        let mut invoice = Invoice::default();
        apply_reply(r#"{"票种": "电子发票（增值税专用发票）"}"#, &mut invoice).unwrap();
        assert_eq!(invoice.kind, Some(InvoiceKind::DigitalSpecial));
    }

    #[test]
    fn large_photos_are_scaled_down_before_upload() {
        let big = DynamicImage::new_rgb8(4000, 3000);
        let attachment = encode_for_upload(&big).unwrap();
        assert_eq!(attachment.mime, "image/jpeg");

        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&attachment.data_base64)
            .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.width().max(decoded.height()), MAX_EDGE);
    }

    /// An image already small enough must not be upscaled - that would add
    /// bytes and interpolation artefacts for no gain.
    #[test]
    fn small_photos_are_left_alone() {
        let small = DynamicImage::new_rgb8(800, 600);
        let attachment = encode_for_upload(&small).unwrap();
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&attachment.data_base64)
            .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (800, 600));
    }

    /// JPEG has no alpha channel; a PNG screenshot with transparency must
    /// still encode rather than failing the whole import.
    #[test]
    fn images_with_an_alpha_channel_still_encode() {
        let rgba = DynamicImage::new_rgba8(200, 200);
        assert!(encode_for_upload(&rgba).is_ok());
    }
}
