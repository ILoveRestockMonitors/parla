//! Complete-candidate insertion and verified replacement contracts.
#[cfg(windows)]
pub mod transaction;
#[cfg(not(windows))]
#[path = "portable.rs"]
pub mod transaction;
