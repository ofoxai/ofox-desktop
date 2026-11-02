//! First-party chat assistant. The host, rather than the model, owns identity,
//! process lifetimes and the persisted conversation files.
pub mod attachments;
pub mod audio;
pub mod audio_input;
pub mod capabilities;
pub mod catalog;
mod context;
mod download;
pub mod drops;
pub mod host;
mod live;
mod media;
mod media_host;
mod media_http;
pub mod media_protocol;
pub mod process;
mod projection;
mod provenance;
mod requests;
pub mod store;
pub mod thinking;
mod title;
pub mod tools;
mod transfers;
pub mod types;
pub mod video;
mod video_host;
mod video_http;
mod web;

pub use host::AssistantHost;

pub mod speech;
