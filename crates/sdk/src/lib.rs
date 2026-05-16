//! # Camouflage — Rust SDK
//!
//! Event-native rendering runtime for long-running AI agent applications.
//!
//! This crate is a thin re-export facade over the workspace's internal
//! crates. Hosts embedding Camouflage should depend on `camouflage` (this
//! crate) — not the internal `camouflage-protocol` / `camouflage-store` /
//! `camouflage-renderer` crates directly, which may rearrange between
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
//! let session = uuid::Uuid::new_v4();
//!
//! // 2. Apply events to a renderer.
//! let mut model = RenderModel::new();
//! let event = Event::new(
//!     session, 0, EventType::SessionStarted, serde_json::json!({}),
//! );
//! store.append_event(&event).unwrap();
//! model.apply(&event);
//!
//! // 3. Optionally project a Snapshot for a remote frontend.
//! let snapshot: Snapshot = model.snapshot();
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
//! - `camouflage-tui` — interactive renderer
//! - `camouflage-record` — persist NDJSON to SQLite, no UI
//! - `camouflage-broadcast` — fan NDJSON to WebSocket clients
//! - `camouflage-export` — dump a stored session to portable NDJSON
//! - `camouflage-validate` — strict NDJSON validator
//! - `camouflage-replay-check` — deterministic CI helper
//!
//! ## Modules
//!
//! - [`protocol`]    — `Event`, `EventType`, payload structs, `Direction`
//! - [`store`]       — `EventStore` trait, `SqliteStore` reference impl
//! - [`renderer`]    — `RenderModel`, `Renderer` trait, `Snapshot`,
//!   markdown helpers, theme registry
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

pub mod renderer {
    //! Re-export of `camouflage_renderer`.
    pub use camouflage_renderer::*;
}

pub mod headless {
    //! Re-export of `camouflage_headless`.
    pub use camouflage_headless::*;
}

pub mod prelude {
    //! Common imports for hosts embedding Camouflage.
    pub use camouflage_protocol::{
        payloads, Direction, Event, EventType, ProtocolError, SCHEMA_VERSION,
    };
    pub use camouflage_renderer::{
        RenderModel, Renderer, Snapshot, SnapshotRenderer, ViewportState,
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
        let session = uuid::Uuid::nil();
        let _ev = Event::new(
            session,
            0,
            EventType::SessionStarted,
            serde_json::json!({}),
        );
        let mut model = RenderModel::new();
        let _snap = model.snapshot();
        let store = SqliteStore::open_in_memory().expect("store");
        assert_eq!(store.latest_seq(session).unwrap_or(-1), -1);
    }

    #[test]
    fn version_constant_matches_workspace() {
        assert!(!super::VERSION.is_empty());
    }
}
