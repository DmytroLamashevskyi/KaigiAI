//! Core persistence service.
//!
//! This module is intentionally free of any Tauri dependency: it only knows
//! about a SQLite connection string and plain data types. The desktop shell
//! (Tauri commands) and a future Axum HTTP/WS server can both drive the same
//! `Db` service — see docs/PROJECT.md §6, §14 ("ядро как сервис").

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub lang_a: String,
    pub lang_b: String,
    /// JSON map of diarization label -> display name (e.g. {"Speaker 1":"Масаки"}).
    /// NULL until the user renames a speaker. See docs/PROJECT.md §10.6.
    pub speaker_names: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub source: String,
    pub detected_lang: String,
    pub speaker: Option<String>,
    pub original_text: String,
    pub translated_text: String,
    /// Secondary translation, only set for "foreign" rows whose `detected_lang`
    /// is outside the conversation's pair (docs/PROJECT.md §10.7, variant A):
    /// `translated_text` holds the `lang_a` translation, this holds `lang_b`.
    /// NULL for ordinary bilingual rows.
    #[serde(default)]
    pub translated_text_b: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub created_at: i64,
    /// Pipeline latency in ms: (message persisted) − (segment emitted by VAD),
    /// i.e. how long STT + translation took (docs/PROJECT.md §10.8). `None` for
    /// text messages and rows created before this column existed.
    #[serde(default)]
    pub processing_ms: Option<i64>,
}

/// One-shot snapshot used to hydrate the frontend on boot.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub conversations: Vec<Conversation>,
    pub messages: HashMap<String, Vec<Message>>,
    pub settings: Option<serde_json::Value>,
}

const SETTINGS_KEY: &str = "app";

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS conversation (
  id          TEXT PRIMARY KEY,
  title       TEXT NOT NULL,
  lang_a      TEXT NOT NULL,
  lang_b      TEXT NOT NULL,
  speaker_names TEXT,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS message (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
  source          TEXT NOT NULL,
  detected_lang   TEXT NOT NULL,
  speaker         TEXT,
  original_text   TEXT NOT NULL,
  translated_text TEXT NOT NULL,
  translated_text_b TEXT,
  start_ms        INTEGER NOT NULL,
  end_ms          INTEGER NOT NULL,
  created_at      INTEGER NOT NULL,
  processing_ms   INTEGER
);
CREATE INDEX IF NOT EXISTS idx_message_conv ON message(conversation_id);

CREATE TABLE IF NOT EXISTS summary (
  conversation_id TEXT PRIMARY KEY REFERENCES conversation(id) ON DELETE CASCADE,
  content         TEXT NOT NULL,
  model           TEXT,
  created_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS audio_clip (
  message_id   TEXT PRIMARY KEY REFERENCES message(id) ON DELETE CASCADE,
  path         TEXT NOT NULL,
  duration_ms  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS note (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
  message_id      TEXT,
  content         TEXT NOT NULL,
  created_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS setting (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Open (creating if missing) a SQLite database at `path` and ensure the
    /// schema exists.
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, sqlx::Error> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await?;

        let db = Self { pool };
        db.init_schema().await?;
        Ok(db)
    }

    async fn init_schema(&self) -> Result<(), sqlx::Error> {
        sqlx::raw_sql(SCHEMA).execute(&self.pool).await?;
        // Migration for DBs created before the column existed. `CREATE TABLE IF
        // NOT EXISTS` won't add columns, so add it idempotently and ignore the
        // "duplicate column" error on already-migrated databases.
        let _ = sqlx::query("ALTER TABLE conversation ADD COLUMN speaker_names TEXT")
            .execute(&self.pool)
            .await;
        // Secondary translation for foreign-language rows (§10.7 variant A).
        let _ = sqlx::query("ALTER TABLE message ADD COLUMN translated_text_b TEXT")
            .execute(&self.pool)
            .await;
        // Speech→text pipeline latency in ms (§10.8).
        let _ = sqlx::query("ALTER TABLE message ADD COLUMN processing_ms INTEGER")
            .execute(&self.pool)
            .await;
        Ok(())
    }

    pub async fn bootstrap(&self) -> Result<Bootstrap, sqlx::Error> {
        let conversations = self.list_conversations().await?;
        let all = self.list_all_messages().await?;
        let mut messages: HashMap<String, Vec<Message>> = HashMap::new();
        for c in &conversations {
            messages.entry(c.id.clone()).or_default();
        }
        for m in all {
            messages.entry(m.conversation_id.clone()).or_default().push(m);
        }
        let settings = self.get_setting(SETTINGS_KEY).await?
            .and_then(|s| serde_json::from_str(&s).ok());
        Ok(Bootstrap { conversations, messages, settings })
    }

    pub async fn list_conversations(&self) -> Result<Vec<Conversation>, sqlx::Error> {
        sqlx::query_as::<_, Conversation>(
            "SELECT id, title, lang_a, lang_b, speaker_names, created_at, updated_at \
             FROM conversation ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_conversation(&self, id: &str) -> Result<Option<Conversation>, sqlx::Error> {
        sqlx::query_as::<_, Conversation>(
            "SELECT id, title, lang_a, lang_b, speaker_names, created_at, updated_at \
             FROM conversation WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn list_messages(&self, conversation_id: &str) -> Result<Vec<Message>, sqlx::Error> {
        sqlx::query_as::<_, Message>(
            "SELECT id, conversation_id, source, detected_lang, speaker, \
             original_text, translated_text, translated_text_b, start_ms, end_ms, created_at, processing_ms \
             FROM message WHERE conversation_id = ? ORDER BY created_at ASC",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await
    }

    async fn list_all_messages(&self) -> Result<Vec<Message>, sqlx::Error> {
        sqlx::query_as::<_, Message>(
            "SELECT id, conversation_id, source, detected_lang, speaker, \
             original_text, translated_text, translated_text_b, start_ms, end_ms, created_at, processing_ms \
             FROM message ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn create_conversation(&self, c: &Conversation) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO conversation (id, title, lang_a, lang_b, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.id)
        .bind(&c.title)
        .bind(&c.lang_a)
        .bind(&c.lang_b)
        .bind(c.created_at)
        .bind(c.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn rename_conversation(&self, id: &str, title: &str, updated_at: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE conversation SET title = ?, updated_at = ? WHERE id = ?")
            .bind(title)
            .bind(updated_at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_conversation_langs(&self, id: &str, lang_a: &str, lang_b: &str, updated_at: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE conversation SET lang_a = ?, lang_b = ?, updated_at = ? WHERE id = ?")
            .bind(lang_a)
            .bind(lang_b)
            .bind(updated_at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Persist the diarization label -> display-name map (JSON) for a conversation.
    pub async fn set_speaker_names(&self, id: &str, names_json: &str, updated_at: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE conversation SET speaker_names = ?, updated_at = ? WHERE id = ?")
            .bind(names_json)
            .bind(updated_at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Reassign a single message to a different speaker label (manual override
    /// of diarization). `label` may be an existing or brand-new label; `None`
    /// clears the attribution. Clustering state is untouched — this is purely a
    /// post-hoc correction. See docs/PROJECT.md §10.9.
    pub async fn set_message_speaker(&self, message_id: &str, label: Option<&str>) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE message SET speaker = ? WHERE id = ?")
            .bind(label)
            .bind(message_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn touch_conversation(&self, id: &str, updated_at: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE conversation SET updated_at = ? WHERE id = ?")
            .bind(updated_at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_conversation(&self, id: &str) -> Result<(), sqlx::Error> {
        // ON DELETE CASCADE (foreign_keys=ON) removes messages/summary/notes.
        sqlx::query("DELETE FROM conversation WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_message(&self, m: &Message) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO message (id, conversation_id, source, detected_lang, speaker, \
             original_text, translated_text, translated_text_b, start_ms, end_ms, created_at, processing_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
             detected_lang = excluded.detected_lang, \
             original_text = excluded.original_text, \
             translated_text = excluded.translated_text, \
             translated_text_b = excluded.translated_text_b, \
             processing_ms = excluded.processing_ms",
        )
        .bind(&m.id)
        .bind(&m.conversation_id)
        .bind(&m.source)
        .bind(&m.detected_lang)
        .bind(&m.speaker)
        .bind(&m.original_text)
        .bind(&m.translated_text)
        .bind(&m.translated_text_b)
        .bind(m.start_ms)
        .bind(m.end_ms)
        .bind(m.created_at)
        .bind(m.processing_ms)
        .execute(&self.pool)
        .await?;
        self.touch_conversation(&m.conversation_id, m.created_at).await?;
        Ok(())
    }

    /// Record an on-disk audio clip for a message (used when `saveAudio` is on).
    pub async fn add_audio_clip(&self, message_id: &str, path: &str, duration_ms: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO audio_clip (message_id, path, duration_ms) VALUES (?, ?, ?) \
             ON CONFLICT(message_id) DO UPDATE SET path = excluded.path, duration_ms = excluded.duration_ms",
        )
        .bind(message_id)
        .bind(path)
        .bind(duration_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// File paths of saved audio clips for a conversation's messages, ordered by
    /// message start time. Used by the ZIP export.
    pub async fn list_audio_clips(&self, conversation_id: &str) -> Result<Vec<(String, String)>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT a.message_id, a.path FROM audio_clip a \
             JOIN message m ON m.id = a.message_id \
             WHERE m.conversation_id = ? ORDER BY m.start_ms ASC",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// The app settings blob as JSON (empty object if never saved).
    pub async fn get_app_settings(&self) -> Result<serde_json::Value, sqlx::Error> {
        let raw = self.get_setting(SETTINGS_KEY).await?;
        Ok(raw
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({})))
    }

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM setting WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }

    pub async fn save_settings(&self, settings: &serde_json::Value) -> Result<(), sqlx::Error> {
        let value = serde_json::to_string(settings).unwrap_or_else(|_| "{}".into());
        sqlx::query(
            "INSERT INTO setting (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(SETTINGS_KEY)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(id: &str, ts: i64) -> Conversation {
        Conversation {
            id: id.into(),
            title: "Untitled".into(),
            lang_a: "ru".into(),
            lang_b: "en".into(),
            speaker_names: None,
            created_at: ts,
            updated_at: ts,
        }
    }

    fn msg(id: &str, conv_id: &str, ts: i64) -> Message {
        Message {
            id: id.into(),
            conversation_id: conv_id.into(),
            source: "text".into(),
            detected_lang: "ru".into(),
            speaker: None,
            original_text: "Привет".into(),
            translated_text: "Hello".into(),
            translated_text_b: None,
            start_ms: 0,
            end_ms: 0,
            processing_ms: None,
            created_at: ts,
        }
    }

    #[test]
    fn roundtrip() {
        let path = std::env::temp_dir()
            .join(format!("kaigi_test_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        tauri::async_runtime::block_on(async {
            let db = Db::connect(&path).await.expect("connect");

            // empty bootstrap
            let boot = db.bootstrap().await.unwrap();
            assert_eq!(boot.conversations.len(), 0);
            assert!(boot.settings.is_none());

            // create conversation
            db.create_conversation(&conv("c1", 100)).await.unwrap();
            let boot = db.bootstrap().await.unwrap();
            assert_eq!(boot.conversations.len(), 1);
            assert_eq!(boot.messages.get("c1").map(|v| v.len()), Some(0));

            // add message touches updated_at and is grouped/listed
            db.add_message(&msg("m1", "c1", 200)).await.unwrap();
            let listed = db.list_messages("c1").await.unwrap();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].translated_text, "Hello");
            let boot = db.bootstrap().await.unwrap();
            assert_eq!(boot.messages.get("c1").map(|v| v.len()), Some(1));
            assert_eq!(boot.conversations[0].updated_at, 200);

            // rename + relang
            db.rename_conversation("c1", "Renamed", 300).await.unwrap();
            db.set_conversation_langs("c1", "ja", "ko", 400).await.unwrap();
            let boot = db.bootstrap().await.unwrap();
            assert_eq!(boot.conversations[0].title, "Renamed");
            assert_eq!(boot.conversations[0].lang_a, "ja");
            assert_eq!(boot.conversations[0].lang_b, "ko");

            // settings persist + read back
            let s = serde_json::json!({ "theme": "dark", "fontSize": "large" });
            db.save_settings(&s).await.unwrap();
            let boot = db.bootstrap().await.unwrap();
            assert_eq!(boot.settings, Some(s));

            // delete cascades to messages
            db.delete_conversation("c1").await.unwrap();
            let boot = db.bootstrap().await.unwrap();
            assert_eq!(boot.conversations.len(), 0);
            assert_eq!(db.list_messages("c1").await.unwrap().len(), 0);
        });

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
