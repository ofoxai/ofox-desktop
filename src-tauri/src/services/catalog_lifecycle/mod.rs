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
    // Only the unix process runner times out or cleans up; other platforms stub it.
    #[cfg_attr(not(unix), allow(dead_code))]
    Timeout,
    Process,
    #[cfg_attr(not(unix), allow(dead_code))]
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
