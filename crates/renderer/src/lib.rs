//! Backend-agnostic viewport state and render model.
//!
//! Pure logic — no terminal I/O. Owns bounded memory only:
//! a ring buffer of recent rendered rows, one active stream buffer,
//! and a small map of active tool states.

pub mod viewport;
pub mod model;
pub mod snapshot;

pub use model::{
    format_elapsed_ms, reconstruct_rows, BackgroundTask, BackgroundTaskState, PendingPermission,
    RenderModel, Row, RowKind, ToolState,
};
pub use viewport::ViewportState;
pub use snapshot::{
    Snapshot, SnapshotPermission, SnapshotRow, SnapshotRowKind, SnapshotTask, SnapshotTaskState,
};

use camouflage_protocol::Event;

/// Contract every renderer (TUI, browser viewer, SDK consumer, future
/// native shell) implements.
///
/// Applies events one at a time and reports whether the renderer's display
/// state changed. Implementations that drive a remote frontend should
/// additionally produce a [`Snapshot`] via [`SnapshotRenderer::snapshot`] so
/// downstream consumers can render without re-deriving protocol semantics.
pub trait Renderer {
    /// Apply one event to the renderer's internal state. Returns `true` if
    /// the display needs to be redrawn / re-broadcast.
    fn apply(&mut self, ev: &Event) -> bool;
}

/// Renderers that can produce a serializable view of their current state.
/// Implemented by `RenderModel`; required for any renderer that drives a
/// remote / non-Rust frontend over a wire protocol.
pub trait SnapshotRenderer: Renderer {
    fn snapshot(&self) -> Snapshot;
}

impl Renderer for RenderModel {
    fn apply(&mut self, ev: &Event) -> bool {
        RenderModel::apply(self, ev)
    }
}

impl SnapshotRenderer for RenderModel {
    fn snapshot(&self) -> Snapshot {
        let rows = self
            .rows()
            .iter()
            .map(|r| SnapshotRow {
                seq: r.seq,
                kind: row_kind_to_snapshot(&r.kind),
                text: r.text.clone(),
                tool_id: r.tool_id.clone(),
            })
            .collect();
        let tasks = self
            .background_tasks()
            .iter()
            .map(|t| SnapshotTask {
                task_id: t.task_id.clone(),
                label: t.label.clone(),
                state: match t.state {
                    BackgroundTaskState::Running => SnapshotTaskState::Running,
                    BackgroundTaskState::Done => SnapshotTaskState::Done,
                    BackgroundTaskState::Error => SnapshotTaskState::Error,
                },
                progress: t.progress,
            })
            .collect();
        let pending_permission = self.pending_permission().map(|p| SnapshotPermission {
            request_id: p.request_id.clone(),
            tool: p.tool.clone(),
            action: p.action.clone(),
            detail: p.detail.clone(),
        });
        Snapshot {
            total_rows: self.total_rows(),
            rows,
            status: self.status_segments().clone(),
            tasks,
            pending_permission,
        }
    }
}

fn row_kind_to_snapshot(k: &RowKind) -> SnapshotRowKind {
    match k {
        RowKind::System => SnapshotRowKind::System,
        RowKind::User => SnapshotRowKind::User,
        RowKind::Assistant => SnapshotRowKind::Assistant,
        RowKind::Tool => SnapshotRowKind::Tool,
        RowKind::Error => SnapshotRowKind::Error,
        RowKind::Marker => SnapshotRowKind::Marker,
    }
}

#[cfg(test)]
mod renderer_trait_tests {
    use super::*;
    use camouflage_protocol::EventType;
    use serde_json::json;
    use uuid::Uuid;

    fn ev(seq: i64, et: EventType, payload: serde_json::Value) -> Event {
        Event {
            id: Uuid::nil(),
            session_id: Uuid::nil(),
            seq,
            timestamp_ms: 0,
            schema_version: 1,
            event_type: et,
            payload,
        }
    }

    #[test]
    fn render_model_implements_renderer_trait() {
        let mut m: Box<dyn Renderer> = Box::new(RenderModel::new());
        let changed = m.apply(&ev(0, EventType::UserMessageCreated, json!({"text":"hi"})));
        assert!(changed);
    }

    #[test]
    fn snapshot_roundtrips_through_json() {
        let mut m = RenderModel::new();
        m.apply(&ev(0, EventType::SessionStarted, json!({})));
        m.apply(&ev(1, EventType::UserMessageCreated, json!({"text":"hello"})));
        m.apply(&ev(
            2,
            EventType::StatusUpdate,
            json!({"segments":{"mode":"edit","phase":"thinking"}}),
        ));
        m.apply(&ev(
            3,
            EventType::BackgroundTaskUpdate,
            json!({"task_id":"skills","label":"indexing","state":"running","progress":0.5}),
        ));
        let snap = m.snapshot();
        assert_eq!(snap.status.get("mode").map(|s| s.as_str()), Some("edit"));
        assert_eq!(snap.tasks.len(), 1);
        assert_eq!(snap.tasks[0].state, SnapshotTaskState::Running);
        assert!(snap.rows.iter().any(|r| matches!(r.kind, SnapshotRowKind::User)));

        let s = serde_json::to_string(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&s).unwrap();
        assert_eq!(snap, back);
    }

    #[test]
    fn snapshot_pending_permission_surfaces() {
        let mut m = RenderModel::new();
        m.apply(&ev(
            0,
            EventType::PermissionRequested,
            json!({"request_id":"perm-1","tool":"edit","action":"apply patch","detail":"3 lines"}),
        ));
        let snap = m.snapshot();
        let p = snap.pending_permission.expect("pending permission present");
        assert_eq!(p.request_id, "perm-1");
        assert_eq!(p.tool, "edit");
    }
}
