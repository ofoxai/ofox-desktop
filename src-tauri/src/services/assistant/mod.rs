//! First-party assistant host. The host, rather than the model, owns identity,
//! permission grants, process lifetimes and the persisted UI projection.
pub(crate) mod backup;
pub(crate) mod capabilities;
pub(crate) mod catalog;
pub(crate) mod host;
pub(crate) mod local_data;
pub(crate) mod policy;
pub(crate) mod process;
pub(crate) mod store;
pub(crate) mod tools;
pub(crate) mod types;
pub(crate) mod web;
pub(crate) use host::AssistantHost;
