//! The Tauri surface the frontend calls.
//!
//! Every command here is thin: it locks the ledger, calls into a pure module
//! ([`crate::extract`], [`crate::classify`], [`crate::report`]), and returns.
//! Nothing that decides an amount lives at this layer, which is what keeps
//! the parts handling money testable without a running app.

pub mod ingest;
pub mod invoices;
pub mod reports;
pub mod rules;
pub mod settings;

use rusqlite::Connection;
use std::sync::Mutex;

/// Shared application state.
///
/// One connection behind a mutex rather than a pool: this is a single-user
/// desktop ledger whose longest operation is a batch import, and a pool would
/// add a failure mode (exhaustion) to solve contention that cannot happen.
/// SQLite runs in WAL mode, so a read is never blocked behind the import's
/// writes for long.
pub struct AppState {
    pub db: Mutex<Connection>,
}

impl AppState {
    /// Locks the ledger, recovering from a poisoned mutex.
    ///
    /// A panic while holding the lock (a corrupt row, a bug in a parser)
    /// poisons it, and the honest response in a desktop app is to carry on
    /// rather than make every subsequent action fail for the rest of the
    /// session. SQLite's own state survives a panic in our code: an
    /// unfinished statement is rolled back, not left half-written.
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }
}
