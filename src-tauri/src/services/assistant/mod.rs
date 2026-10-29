//! First-party chat assistant. The host, rather than the model, owns identity,
//! process lifetimes and the persisted conversation files.
pub mod attachments;
pub mod catalog;
pub mod drops;
pub mod host;
mod media;
mod media_host;
mod media_http;
pub mod media_protocol;
pub mod process;
mod projection;
mod requests;
pub mod store;
pub mod thinking;
mod title;
mod transfers;
pub mod types;
pub mod video;
mod video_host;
mod video_http;
mod web;

pub use host::AssistantHost;
