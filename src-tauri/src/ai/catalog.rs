//! The provider catalog: one entry per supported AI service, as data rather
//! than scattered `match provider.as_str()` special cases.
//!
//! Mirrored on the TS side by `src/settings/provider-catalog.ts` (kept in
//! sync by hand - comments on both sides point here).
//!
//! ## Why every entry is a Chinese provider
//!
//! ZhiShui reads Chinese invoices, for people in China. That shapes the list
//! three ways:
//!
//! - **Reachability.** OpenAI and Anthropic endpoints do not resolve from a
//!   mainland network without a proxy. A default that fails to connect for
//!   most users is not a default.
//! - **Recognition quality.** The task is dense Chinese document OCR - 8pt
//!   宋体, a red 发票专用章 overprinting the text, 20-digit numbers.
//!   Domestic vision models are trained on exactly this material, and Qwen
//!   ships an OCR-specialised variant of its VL model.
//! - **Cost.** One photographed invoice is one image and a few hundred output
//!   tokens. At domestic pricing that is a fraction of a 分 per invoice,
//!   which is what makes the vision fallback usable in bulk at all.
//!
//! Every one of them exposes an OpenAI **Chat Completions** compatible
//! endpoint, so there is one dialect and one auth method (API key) - see the
//! `aicompat` crate docs.

use serde::Serialize;

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCatalogEntry {
    /// Matches the `provider` string every `ai_*` command dispatches on.
    pub id: &'static str,
    /// The provider's own Chinese name, as its users know it.
    pub label: &'static str,
    /// Where to get a key, shown beside the empty key field. Leaving a user
    /// to find this themselves is the most common place a setup stalls.
    pub console_url: &'static str,
    /// Fixed endpoint; None only for "custom", where the user supplies it.
    pub base_url: Option<&'static str>,
    /// The model used to READ an invoice photo. None means this provider has
    /// no vision model and is offered only for category suggestions.
    pub vision_model: Option<&'static str>,
    /// The cheap text model used for category suggestions.
    pub text_model: Option<&'static str>,
    /// True for local servers (Ollama) that work without a key.
    pub key_optional: bool,
    /// Whether a live `GET /models` call works. False hides the
    /// 「获取模型列表」button rather than offering one that can only fail.
    pub models_listable: bool,
    /// A few current model ids, so the picker is never empty before - or
    /// instead of - a live fetch.
    pub known_models: &'static [&'static str],
}

impl ProviderCatalogEntry {
    pub fn supports_vision(&self) -> bool {
        self.vision_model.is_some()
    }
}

pub const PROVIDER_CATALOG: &[ProviderCatalogEntry] = &[
    // Alibaba's DashScope in OpenAI-compatible mode. First in the list
    // because `qwen-vl-ocr` is purpose-built for this exact job - dense
    // Chinese document text - rather than a general vision model asked to
    // read.
    ProviderCatalogEntry {
        id: "qwen",
        label: "阿里云百炼（通义千问）",
        console_url: "https://bailian.console.aliyun.com/",
        base_url: Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        vision_model: Some("qwen-vl-ocr-latest"),
        text_model: Some("qwen-flash"),
        key_optional: false,
        models_listable: true,
        known_models: &[
            "qwen-vl-ocr-latest",
            "qwen-vl-max-latest",
            "qwen-vl-plus-latest",
            "qwen-flash",
            "qwen-plus",
        ],
    },
    ProviderCatalogEntry {
        id: "zhipu",
        label: "智谱 GLM",
        console_url: "https://bigmodel.cn/usercenter/apikeys",
        base_url: Some("https://open.bigmodel.cn/api/paas/v4"),
        vision_model: Some("glm-4v-flash"),
        text_model: Some("glm-4-flash"),
        key_optional: false,
        models_listable: false,
        known_models: &["glm-4v-flash", "glm-4v-plus", "glm-4-flash", "glm-4-air"],
    },
    ProviderCatalogEntry {
        id: "volcengine",
        label: "火山方舟（豆包）",
        console_url: "https://console.volcengine.com/ark",
        // Ark addresses models by 接入点 ID (`ep-...`), not by name, so this
        // default is a starting point the user replaces with their own
        // endpoint id - the settings pane says as much.
        base_url: Some("https://ark.cn-beijing.volces.com/api/v3"),
        vision_model: Some("doubao-vision-pro-32k"),
        text_model: Some("doubao-lite-32k"),
        key_optional: false,
        models_listable: false,
        known_models: &["doubao-vision-pro-32k", "doubao-lite-32k"],
    },
    ProviderCatalogEntry {
        id: "moonshot",
        label: "月之暗面 Kimi",
        console_url: "https://platform.moonshot.cn/console/api-keys",
        base_url: Some("https://api.moonshot.cn/v1"),
        vision_model: Some("moonshot-v1-8k-vision-preview"),
        text_model: Some("moonshot-v1-8k"),
        key_optional: false,
        models_listable: true,
        known_models: &[
            "moonshot-v1-8k-vision-preview",
            "moonshot-v1-8k",
            "moonshot-v1-32k",
        ],
    },
    ProviderCatalogEntry {
        id: "stepfun",
        label: "阶跃星辰 StepFun",
        console_url: "https://platform.stepfun.com/",
        base_url: Some("https://api.stepfun.com/v1"),
        vision_model: Some("step-1v-8k"),
        text_model: Some("step-1-8k"),
        key_optional: false,
        models_listable: true,
        known_models: &["step-1v-8k", "step-1v-32k", "step-1-8k"],
    },
    ProviderCatalogEntry {
        id: "hunyuan",
        label: "腾讯混元",
        console_url: "https://console.cloud.tencent.com/hunyuan/api-key",
        base_url: Some("https://api.hunyuan.cloud.tencent.com/v1"),
        vision_model: Some("hunyuan-vision"),
        text_model: Some("hunyuan-lite"),
        key_optional: false,
        models_listable: false,
        known_models: &["hunyuan-vision", "hunyuan-lite", "hunyuan-standard"],
    },
    ProviderCatalogEntry {
        id: "minimax",
        label: "MiniMax",
        console_url: "https://platform.minimaxi.com/user-center/basic-information",
        base_url: Some("https://api.minimaxi.com/v1"),
        vision_model: Some("MiniMax-VL-01"),
        text_model: Some("MiniMax-Text-01"),
        key_optional: false,
        models_listable: false,
        known_models: &["MiniMax-VL-01", "MiniMax-Text-01"],
    },
    // An aggregator. It earns its place by hosting open-weight Qwen-VL
    // checkpoints, which is a second route to the best model for this job.
    ProviderCatalogEntry {
        id: "siliconflow",
        label: "硅基流动 SiliconFlow",
        console_url: "https://cloud.siliconflow.cn/account/ak",
        base_url: Some("https://api.siliconflow.cn/v1"),
        vision_model: Some("Qwen/Qwen2.5-VL-72B-Instruct"),
        text_model: Some("Qwen/Qwen2.5-7B-Instruct"),
        key_optional: false,
        models_listable: true,
        known_models: &[
            "Qwen/Qwen2.5-VL-72B-Instruct",
            "Qwen/Qwen2.5-VL-32B-Instruct",
            "Qwen/Qwen2.5-7B-Instruct",
        ],
    },
    // Text only - DeepSeek ships no vision model on its API. Listed anyway
    // because it is very cheap and category suggestion is a text task.
    ProviderCatalogEntry {
        id: "deepseek",
        label: "深度求索 DeepSeek",
        console_url: "https://platform.deepseek.com/api_keys",
        base_url: Some("https://api.deepseek.com"),
        vision_model: None,
        text_model: Some("deepseek-chat"),
        key_optional: false,
        models_listable: true,
        known_models: &["deepseek-chat", "deepseek-reasoner"],
    },
    // Local, and therefore the only option that keeps invoice photos on the
    // machine entirely. Whether it can read one depends on the model pulled.
    ProviderCatalogEntry {
        id: "ollama",
        label: "Ollama（本机）",
        console_url: "https://ollama.com/",
        base_url: Some("http://localhost:11434/v1"),
        vision_model: Some("qwen2.5vl:7b"),
        text_model: Some("qwen2.5:7b"),
        key_optional: true,
        models_listable: true,
        known_models: &["qwen2.5vl:7b", "qwen2.5vl:3b", "qwen2.5:7b"],
    },
    ProviderCatalogEntry {
        id: "custom",
        label: "自定义接口",
        console_url: "",
        base_url: None,
        vision_model: None,
        text_model: None,
        key_optional: true,
        models_listable: true,
        known_models: &[],
    },
];

pub fn find(provider: &str) -> Option<&'static ProviderCatalogEntry> {
    PROVIDER_CATALOG.iter().find(|entry| entry.id == provider)
}

#[tauri::command]
pub fn list_providers() -> Vec<ProviderCatalogEntry> {
    PROVIDER_CATALOG.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_are_unique() {
        let mut ids: Vec<&str> = PROVIDER_CATALOG.iter().map(|e| e.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "provider id 重复");
    }

    /// Every entry but "custom" has to name its endpoint, or the request has
    /// nowhere to go and the failure surfaces as a confusing network error.
    #[test]
    fn every_fixed_provider_carries_a_base_url_and_models() {
        for entry in PROVIDER_CATALOG {
            if entry.id != "custom" {
                assert!(entry.base_url.is_some(), "{} 缺少 base_url", entry.id);
                assert!(!entry.known_models.is_empty(), "{} 缺少模型", entry.id);
            }
        }
    }

    /// The whole point of the list: at least one usable vision provider, or
    /// photographed invoices cannot be read at all.
    #[test]
    fn at_least_one_provider_can_read_a_photo() {
        assert!(PROVIDER_CATALOG.iter().any(|e| e.supports_vision()));
    }

    /// A declared default must be selectable in the picker, or changing the
    /// model away from it is a one-way door.
    #[test]
    fn declared_models_appear_in_the_picker() {
        for entry in PROVIDER_CATALOG.iter().filter(|e| e.id != "custom") {
            for model in [entry.vision_model, entry.text_model].into_iter().flatten() {
                assert!(
                    entry.known_models.contains(&model),
                    "{} 的默认模型 {model} 不在 known_models 中",
                    entry.id
                );
            }
        }
    }

    #[test]
    fn a_key_is_required_everywhere_except_local_and_custom() {
        for entry in PROVIDER_CATALOG {
            let expected = matches!(entry.id, "ollama" | "custom");
            assert_eq!(entry.key_optional, expected, "{}", entry.id);
        }
    }
}
