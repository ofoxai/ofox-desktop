//! Installation-only management; this module never binds a model or account.
mod jobs;
pub(crate) mod operation;
mod process;
mod provenance;
mod receipt;
mod release;
mod runtime;
mod state;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Failure {
    InvalidPackage,
    Changed,
    Limit,
    Unreadable,
    Cancelled,
    Timeout,
    Process,
    Cleanup,
}
impl Failure {
    fn code(&self) -> &'static str {
        match self {
            Self::InvalidPackage => "invalidPackage",
            Self::Changed => "changed",
            Self::Limit => "limit",
            Self::Unreadable => "unreadable",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::Process => "process",
            Self::Cleanup => "cleanup",
        }
    }
}
