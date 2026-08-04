//! Provider-agnostic API client logic for ZhiShui's AI features.
//!
//! This crate knows nothing about Tauri commands or credential storage - it
//! just knows how to call a provider's API. The main crate wraps it with
//! `#[tauri::command]`s and decides where keys get persisted.
//!
//! Adapted from the crate of the same name in CatVinci's Levis, cut down to
//! what an invoice tool needs. Two things were removed outright, for the
//! same reason:
//!
//! - **The OAuth flows** (ChatGPT / Claude browser sign-in). Every provider
//!   ZhiShui supports is a Chinese one, and they all authenticate with an
//!   API key. A PKCE login no UI can reach is dead code in a signed binary.
//! - **The Responses and Anthropic Messages dialects.** Every provider in
//!   the catalog exposes an OpenAI **Chat Completions** compatible endpoint -
//!   Alibaba DashScope, Zhipu, Volcengine Ark, Moonshot, DeepSeek, Tencent
//!   Hunyuan, StepFun, MiniMax, SiliconFlow. One dialect covers all of them,
//!   so carrying three meant carrying two for nobody.
//!
//! What is left is the part that does the work: the turn model, the tool
//! loop, and one HTTP client that speaks Chat Completions.

pub mod agent;
pub mod http;
pub mod providers;
