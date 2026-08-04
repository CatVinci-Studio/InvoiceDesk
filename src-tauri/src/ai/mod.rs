//! Everything that talks to an AI provider.
//!
//! Two features, two switches, both off until the user turns them on:
//!
//! - [`vision`]     reading an invoice photo the offline layers could not
//! - [`categorize`] proposing a 报销类别 no rule matched
//!
//! [`catalog`] is the list of providers (all Chinese - see its docs) and
//! [`route`] resolves one plus the stored key into a request target.
pub mod catalog;
pub mod categorize;
pub mod route;
pub mod vision;
