//! First-party chat assistant. The host, rather than the model, owns identity,
//! process lifetimes and the persisted conversation files.
pub mod attachments;
pub mod catalog;
pub mod host;
pub mod process;
mod projection;
mod requests;
pub mod store;
pub mod types;

pub use host::AssistantHost;
