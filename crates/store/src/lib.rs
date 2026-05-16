//! SQLite event persistence (WAL, append-only).

use camouflage_protocol::{Event, EventType};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Uuid(#[from] uuid::Error),
    #[error("unknown event type: {0}")]
    UnknownEventType(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub trait EventStore: Send + Sync {
    fn append_event(&self, event: &Event) -> Result<()>;
    fn append_batch(&self, events: &[Event]) -> Result<()>;
    fn load_session(&self, session_id: Uuid) -> Result<Vec<Event>>;
    fn load_range(&self, session_id: Uuid, from_seq: i64, to_seq: i64) -> Result<Vec<Event>>;
    fn latest_seq(&self, session_id: Uuid) -> Result<i64>;
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(dir) = path.as_ref().parent() {
            if !dir.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(dir);
            }
        }
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn init(conn: &Connection) -> Result<()> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS events (
                id              TEXT PRIMARY KEY,
                session_id      TEXT NOT NULL,
                seq             INTEGER NOT NULL,
                timestamp_ms    INTEGER NOT NULL,
                schema_version  INTEGER NOT NULL,
                event_type      TEXT NOT NULL,
                payload         TEXT NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_events_session_seq
                ON events(session_id, seq);
            "#,
        )?;
        Ok(())
    }
}

fn row_to_event(
    id: String,
    session_id: String,
    seq: i64,
    timestamp_ms: i64,
    schema_version: u32,
    event_type: String,
    payload: String,
) -> Result<Event> {
    let et = parse_event_type(&event_type)?;
    Ok(Event {
        id: Uuid::parse_str(&id)?,
        session_id: Uuid::parse_str(&session_id)?,
        seq,
        timestamp_ms,
        schema_version,
        event_type: et,
        payload: serde_json::from_str(&payload)?,
    })
}

fn parse_event_type(s: &str) -> Result<EventType> {
    let v = serde_json::from_str::<EventType>(&format!("\"{}\"", s))
        .map_err(|_| StoreError::UnknownEventType(s.to_string()))?;
    Ok(v)
}

impl EventStore for SqliteStore {
    fn append_event(&self, event: &Event) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO events (id, session_id, seq, timestamp_ms, schema_version, event_type, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event.id.to_string(),
                event.session_id.to_string(),
                event.seq,
                event.timestamp_ms,
                event.schema_version,
                event.event_type.as_str(),
                serde_json::to_string(&event.payload)?,
            ],
        )?;
        Ok(())
    }

    fn append_batch(&self, events: &[Event]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO events (id, session_id, seq, timestamp_ms, schema_version, event_type, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for event in events {
                stmt.execute(params![
                    event.id.to_string(),
                    event.session_id.to_string(),
                    event.seq,
                    event.timestamp_ms,
                    event.schema_version,
                    event.event_type.as_str(),
                    serde_json::to_string(&event.payload)?,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn load_session(&self, session_id: Uuid) -> Result<Vec<Event>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, seq, timestamp_ms, schema_version, event_type, payload
             FROM events WHERE session_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![session_id.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, sid, seq, ts, sv, et, pl) = row?;
            out.push(row_to_event(id, sid, seq, ts, sv, et, pl)?);
        }
        Ok(out)
    }

    fn load_range(&self, session_id: Uuid, from_seq: i64, to_seq: i64) -> Result<Vec<Event>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, seq, timestamp_ms, schema_version, event_type, payload
             FROM events
             WHERE session_id = ?1 AND seq >= ?2 AND seq < ?3
             ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![session_id.to_string(), from_seq, to_seq], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, sid, seq, ts, sv, et, pl) = row?;
            out.push(row_to_event(id, sid, seq, ts, sv, et, pl)?);
        }
        Ok(out)
    }

    fn latest_seq(&self, session_id: Uuid) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let v: Option<i64> = conn
            .query_row(
                "SELECT MAX(seq) FROM events WHERE session_id = ?1",
                params![session_id.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(v.unwrap_or(-1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camouflage_protocol::{Event, EventType, SCHEMA_VERSION};
    use serde_json::json;

    fn ev(sid: Uuid, seq: i64, et: EventType) -> Event {
        Event {
            id: Uuid::new_v4(),
            session_id: sid,
            seq,
            timestamp_ms: seq,
            schema_version: SCHEMA_VERSION,
            event_type: et,
            payload: json!({"seq": seq}),
        }
    }

    #[test]
    fn append_and_load() {
        let s = SqliteStore::open_in_memory().unwrap();
        let sid = Uuid::new_v4();
        let e = ev(sid, 0, EventType::SessionStarted);
        s.append_event(&e).unwrap();
        let loaded = s.load_session(sid).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], e);
    }

    #[test]
    fn batch_and_range() {
        let s = SqliteStore::open_in_memory().unwrap();
        let sid = Uuid::new_v4();
        let batch: Vec<Event> = (0..100)
            .map(|i| ev(sid, i, EventType::AssistantTokenDelta))
            .collect();
        s.append_batch(&batch).unwrap();
        assert_eq!(s.latest_seq(sid).unwrap(), 99);
        let mid = s.load_range(sid, 10, 20).unwrap();
        assert_eq!(mid.len(), 10);
        assert_eq!(mid[0].seq, 10);
        assert_eq!(mid[9].seq, 19);
    }

    #[test]
    fn replay_order_deterministic() {
        let s = SqliteStore::open_in_memory().unwrap();
        let sid = Uuid::new_v4();
        let batch: Vec<Event> = (0..500)
            .map(|i| ev(sid, i, EventType::AssistantTokenDelta))
            .collect();
        s.append_batch(&batch).unwrap();
        let loaded = s.load_session(sid).unwrap();
        for (i, e) in loaded.iter().enumerate() {
            assert_eq!(e.seq, i as i64);
        }
    }

    #[test]
    fn latest_seq_empty_returns_neg_one() {
        let s = SqliteStore::open_in_memory().unwrap();
        assert_eq!(s.latest_seq(Uuid::new_v4()).unwrap(), -1);
    }

    #[test]
    fn insert_100k_events() {
        let s = SqliteStore::open_in_memory().unwrap();
        let sid = Uuid::new_v4();
        let batch: Vec<Event> = (0..100_000)
            .map(|i| ev(sid, i, EventType::AssistantTokenDelta))
            .collect();
        s.append_batch(&batch).unwrap();
        assert_eq!(s.latest_seq(sid).unwrap(), 99_999);
    }
}
