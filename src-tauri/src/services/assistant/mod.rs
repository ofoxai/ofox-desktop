//! First-party chat assistant. The host, rather than the model, owns identity,
//! process lifetimes and the persisted conversation files.
pub mod catalog;
pub mod process;
pub mod store;
pub mod types;
