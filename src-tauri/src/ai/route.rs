//! Turning a provider id plus stored settings into a concrete request target.
//!
//! Isolates "which endpoint, which key, which model" so [`super::vision`] and
//! [`super::categorize`] deal in a resolved [`Endpoint`] and never in
//! per-provider special cases.
//!
//! ## Where each piece of configuration lives, and why
//!
//! - **Settings** (provider, models, whether the vision fallback is on) go in
//!   the SQLite ledger, next to everything else the user configured.
//! - **API keys** deliberately do NOT. They live in a separate 0600 file in
//!   the config directory (see [`crate::auth::keys`]), because the ledger is
//!   a document the user may copy to another machine, put in a backup, or
//!   hand to a colleague along with the invoices - and a credential must not
//!   ride along inside it.

use super::catalog;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// What a resolved request needs.
pub struct Endpoint {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

/// Which model to pick: the one that reads pictures, or the cheap one that
/// answers text questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Vision,
    Text,
}

/// The AI half of the app's settings. Stored as JSON under the `ai` key of
/// the settings table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiSettings {
    /// Whether an unreadable photo may be sent to the provider.
    ///
    /// **Off by default, and that is a deliberate product decision, not
    /// caution for its own sake.** Turning it on means invoice images -
    /// carrying the buyer's name, tax ID and amounts - leave the machine. The
    /// user has to choose that knowingly, so the app never makes the choice
    /// for them by defaulting to convenient.
    pub vision_enabled: bool,
    /// Whether an invoice no rule matched may be sent (as text) for a
    /// category suggestion. Separate switch: this sends a seller name and a
    /// line item, not a picture of the whole invoice, so a user may
    /// reasonably want one and not the other.
    pub suggest_enabled: bool,
    pub provider: String,
    /// Overrides the catalog default when set.
    pub vision_model: Option<String>,
    pub text_model: Option<String>,
    /// Only for `provider == "custom"`.
    pub custom_base_url: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            vision_enabled: false,
            suggest_enabled: false,
            provider: "qwen".to_string(),
            vision_model: None,
            text_model: None,
            custom_base_url: None,
        }
    }
}

pub const NOT_CONFIGURED: &str = "还没有配置 AI 服务商，请在「设置 → AI 识别」中填写 API Key。";
const NO_VISION: &str = "当前服务商没有可用的视觉模型，无法识别照片。请在设置中换一个服务商。";

/// Resolves the endpoint for one request, or explains what is missing.
pub fn resolve(
    app: &AppHandle,
    settings: &AiSettings,
    purpose: Purpose,
) -> Result<Endpoint, String> {
    let entry = catalog::find(&settings.provider)
        .ok_or_else(|| format!("未知的服务商：{}", settings.provider))?;

    let base_url = match entry.base_url {
        Some(url) => url.to_string(),
        None => settings
            .custom_base_url
            .clone()
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| "自定义接口还没有填写地址。".to_string())?,
    };

    let api_key = crate::auth::keys::load_provider_key(app, entry.id)?;
    if api_key.is_none() && !entry.key_optional {
        return Err(NOT_CONFIGURED.to_string());
    }

    let configured = match purpose {
        Purpose::Vision => settings.vision_model.as_deref(),
        Purpose::Text => settings.text_model.as_deref(),
    }
    .map(str::trim)
    .filter(|m| !m.is_empty());

    let model = match configured {
        Some(model) => model.to_string(),
        None => match purpose {
            Purpose::Vision => entry.vision_model.ok_or_else(|| NO_VISION.to_string())?,
            Purpose::Text => entry
                .text_model
                .ok_or_else(|| "当前服务商没有默认模型，请在设置中填写模型名。".to_string())?,
        }
        .to_string(),
    };

    Ok(Endpoint {
        base_url,
        api_key,
        model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_both_network_features_off() {
        let settings = AiSettings::default();
        assert!(!settings.vision_enabled, "视觉识别默认必须关闭");
        assert!(!settings.suggest_enabled, "分类建议默认必须关闭");
    }

    /// Settings written before a field existed must keep deserialising, or an
    /// upgrade silently resets the user's provider choice.
    #[test]
    fn older_settings_json_still_loads() {
        let settings: AiSettings =
            serde_json::from_str(r#"{"provider":"zhipu"}"#).expect("向后兼容");
        assert_eq!(settings.provider, "zhipu");
        assert!(!settings.vision_enabled);
        assert_eq!(settings.vision_model, None);
    }

    #[test]
    fn settings_round_trip_through_json() {
        let settings = AiSettings {
            vision_enabled: true,
            provider: "moonshot".into(),
            vision_model: Some("moonshot-v1-8k-vision-preview".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: AiSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider, "moonshot");
        assert!(back.vision_enabled);
        assert_eq!(
            back.vision_model.as_deref(),
            Some("moonshot-v1-8k-vision-preview")
        );
    }
}
