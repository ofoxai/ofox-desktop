//! Host-owned local operations and authorization.
pub mod approval;
pub mod commands;
pub mod context;
pub mod db_state;
pub mod diagnostics;
pub mod editing;
pub mod events;
pub mod execute;
pub mod files;
pub mod journal;
pub mod policy;
pub mod registry;
pub mod scope;
pub mod transaction;

#[cfg(all(test, target_os = "macos"))]
mod acceptance;
