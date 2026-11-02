//! # Camouflage — Rust SDK
//!
//! Event-native rendering runtime for long-running AI agent applications.
//!
//! This crate is a thin re-export facade over the workspace's internal
//! crates. Hosts embedding Camouflage should depend on `camouflage` (this
//! crate) — not the internal `camouflage-protocol` / `camouflage-store` /
//! `camouflage-inline` crates directly, which may rearrange between
//! versions.
//!
//! ## Quick start (embedding the renderer in a Rust host)
//!
//! ```no_run
//! use camouflage::prelude::*;
//!
//! // 1. Persist events to a store (the architecture invariant is
//! //    "persist before render/broadcast").
//! let store = SqliteStore::open("session.db").unwrap();
//! let session_id = uuid::Uuid::new_v4();
//!
//! // 2. Apply events to the inline renderer.
//! let mut session = Session::new("my-app", Theme::dark(), 80, 24);
//! let event = Event::new(
//!     session_id, 0, EventType::SessionStarted, serde_json::json!({}),
//! );
//! store.append_event(&event).unwrap();
//! session.apply(&event, 0);
//!
//! // 3. Print finished blocks into the terminal's scrollback.
//! for line in session.take_history(0) {
//!     println!("{}", line.plain_text());
//! }
//! ```
//!
//! ## Wire protocol (for non-Rust hosts)
//!
//! Camouflage events are line-delimited JSON. A host adapter just needs
//! to write one `{"event_type": "...", "payload": {...}}` per line to
//! stdout, then pipe through `camouflage-tui --stdin-events`. See
//! `docs/protocol.md` for the full event/payload reference.
//!
//! ## Cargo binaries the workspace ships
//!
//! - `camouflage-tui` — interactive inline renderer, replay and transcript view
//! - `camouflage-record` — persist NDJSON to SQLite, no UI
//! - `camouflage-broadcast` — fan NDJSON to WebSocket clients
//! - `camouflage-export` — dump a stored session to portable NDJSON
//! - `camouflage-validate` — strict NDJSON validator
//!
//! ## Modules
//!
//! - [`protocol`]    — `Event`, `EventType`, payload structs, `Direction`
//! - [`store`]       — `EventStore` trait, `SqliteStore` reference impl
//! - [`inline`]      — `Session`, blocks, markdown, theme, terminal writer
//! - [`headless`]    — NDJSON decoder, strict validator, fixture loader
//! - [`prelude`]     — common imports

pub mod protocol {
    //! Re-export of `camouflage_protocol`.
    pub use camouflage_protocol::*;
}

pub mod store {
    //! Re-export of `camouflage_store`.
    pub use camouflage_store::*;
}

pub mod inline {
    //! Re-export of `camouflage_inline`.
    pub use camouflage_inline::*;
}

pub mod headless {
    //! Re-export of `camouflage_headless`.
    pub use camouflage_headless::*;
}

pub mod prelude {
    //! Common imports for hosts embedding Camouflage.
    pub use camouflage_inline::{Session, Theme};
    pub use camouflage_protocol::{
        payloads, Direction, Event, EventType, ProtocolError, SCHEMA_VERSION,
    };
    pub use camouflage_store::{EventStore, SqliteStore};
}

/// Version of the Rust SDK facade — matches the workspace version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::prelude::*;

    #[test]
    fn prelude_imports_are_usable() {
        let session_id = uuid::Uuid::nil();
        let ev = Event::new(
            session_id,
            0,
            EventType::SessionStarted,
            serde_json::json!({}),
        );
        let mut session = Session::new("test", Theme::dark(), 80, 24);
        session.apply(&ev, 0);
        let store = SqliteStore::open_in_memory().expect("store");
        assert_eq!(store.latest_seq(session_id).unwrap_or(-1), -1);
    }

    #[test]
    fn version_constant_matches_workspace() {
        assert!(!super::VERSION.is_empty());
    }
}
