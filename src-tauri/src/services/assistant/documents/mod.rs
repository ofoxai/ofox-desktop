//! Deliverable documents: where the bundled engines are, how they run, and
//! the `create_document` tool that uses them.
pub mod create;
pub mod engines;
mod fix;
pub(crate) mod math_preview;
pub(crate) mod preview;
pub mod runner;

/// The tool a chat model makes a document with.
pub const CREATE_DOCUMENT: &str = "create_document";

pub fn is_document_tool(name: &str) -> bool {
    name == CREATE_DOCUMENT
}
