#![allow(non_snake_case)]

mod auth;
mod balance;
#[cfg(target_os = "macos")]
mod chatgpt_app;
mod chatgpt_updates;
mod codex_oauth;
mod coding_plan;
mod config;
mod copilot;
mod deeplink;
mod env;
mod failover;
mod global_proxy;
mod hermes;
mod import_export;
mod install_tool;
mod launch_tool;
mod mcp;
mod misc;
mod model_fetch;
pub(crate) mod ofox_apex;
pub(crate) mod ofox_api_keys;
pub(crate) mod ofox_auth;
mod omo;
mod openclaw;
mod plugin;
mod prompt;
mod provider;
mod proxy;
mod session_manager;
mod settings;
pub mod skill;
mod stream_check;
mod subscription;
mod sync_support;
pub(crate) mod tool_update;
#[cfg(target_os = "windows")]
mod windows_chatgpt;

mod lightweight;
pub(crate) mod manage_tool;
mod usage;
mod webdav_sync;
mod window;
mod workspace;

pub use auth::*;
pub use balance::*;
#[cfg(target_os = "macos")]
pub use chatgpt_app::install_chatgpt_desktop_app_with;
pub use codex_oauth::*;
pub use coding_plan::*;
pub use config::*;
pub use copilot::*;
pub use deeplink::*;
pub use env::*;
pub use failover::*;
pub use global_proxy::*;
pub use hermes::*;
pub use import_export::*;
pub use install_tool::*;
pub use launch_tool::*;
pub use mcp::*;
pub use misc::*;
pub use model_fetch::*;
pub use tool_update::*;
#[cfg(target_os = "windows")]
pub use windows_chatgpt::{install_chatgpt_desktop_app_with, upgrade_chatgpt_desktop_app_with};
// Note: ofox_auth items are accessed via commands::ofox_auth:: to avoid
// shadowing lib.rs's top-level ofox_auth module
pub use omo::*;
pub use openclaw::*;
pub use plugin::*;
pub use prompt::*;
pub use provider::*;
pub use proxy::*;
pub use session_manager::*;
pub use settings::*;
pub use skill::*;
pub use stream_check::*;
pub use subscription::*;

pub use lightweight::*;
pub use manage_tool::*;
pub use usage::*;
pub use webdav_sync::*;
pub use window::*;
pub use workspace::*;
