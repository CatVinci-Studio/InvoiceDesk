//! Credential storage.
//!
//! Only API keys, because every provider in the catalog authenticates with
//! one - the OAuth flows Levis carries were dropped along with the providers
//! that needed them (see the `aicompat` crate docs).
pub mod custom_endpoint;
pub mod keys;
