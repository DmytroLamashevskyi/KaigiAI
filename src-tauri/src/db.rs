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
use sqlx::Row;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub lang_a: String,
    pub lang_b: String,
    /// Ordered conversation languages (§10.7). Read from the `langs` JSON column,
    /// falling back to `[lang_a, lang_b]` for older rows (see `row_to_conversation`).
    #[serde(default)]
    pub langs: Vec<String>,
    /// JSON map of diarization label -> display name (e.g. {"Speaker 1":"Масаки"}).
    /// NULL until the user renames a speaker. See docs/PROJECT.md §10.6.
    pub speaker_names: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Per-language translations (lang -> text) for the N-language mode (§10.7),
    /// loaded from `message_translation`. Empty for 2-language rows (which use
    /// translated_text/translated_text_b) and pre-feature messages. Not a column —
    /// filled in by `row_to_message` + `attach_translations`.
    #[serde(default)]
    pub translations: HashMap<String, String>,
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
  -- JSON array of ISO codes for the multi-language mode (§10.7); NULL on older
  -- rows, in which case [lang_a, lang_b] is used. Order = UI column order.
  langs       TEXT,
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

-- Per-language translations of a message (§10.7 N-language mode): one row per
-- conversation language EXCEPT the original. The 2-language path also keeps
-- translated_text/translated_text_b for back-compat; the N-language grid reads
-- this table.
CREATE TABLE IF NOT EXISTS message_translation (
  message_id TEXT NOT NULL REFERENCES message(id) ON DELETE CASCADE,
  lang       TEXT NOT NULL,
  text       TEXT NOT NULL,
  PRIMARY KEY (message_id, lang)
);

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

/// Conversation SELECT column list — the single source of truth shared by every
/// conversation query, so adding a column can't silently miss one site (a
/// mismatch with [`row_to_conversation`] would be a runtime `Row::get` panic).
const CONV_COLS: &str =
    "id, title, lang_a, lang_b, langs, speaker_names, created_at, updated_at";

/// Message SELECT column list — see [`CONV_COLS`] for why this is shared.
const MSG_COLS: &str = "id, conversation_id, source, detected_lang, speaker, \
     original_text, translated_text, translated_text_b, start_ms, end_ms, created_at, processing_ms";

/// Build a [`Conversation`] from a row, parsing the `langs` JSON column (falling
/// back to `[lang_a, lang_b]` for older rows that predate multi-language).
fn row_to_conversation(row: &sqlx::sqlite::SqliteRow) -> Conversation {
    let lang_a: String = row.get("lang_a");
    let lang_b: String = row.get("lang_b");
    let langs = row
        .get::<Option<String>, _>("langs")
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![lang_a.clone(), lang_b.clone()]);
    Conversation {
        id: row.get("id"),
        title: row.get("title"),
        lang_a,
        lang_b,
        langs,
        speaker_names: row.get("speaker_names"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// Build a [`Message`] from a row. `translations` starts empty and is filled in
/// by [`attach_translations`] (mapped manually because `HashMap`/`Vec` fields
/// can't be `sqlx::FromRow`-decoded).
fn row_to_message(row: &sqlx::sqlite::SqliteRow) -> Message {
    Message {
        id: row.get("id"),
        conversation_id: row.get("conversation_id"),
        source: row.get("source"),
        detected_lang: row.get("detected_lang"),
        speaker: row.get("speaker"),
        original_text: row.get("original_text"),
        translated_text: row.get("translated_text"),
        translated_text_b: row.get("translated_text_b"),
        start_ms: row.get("start_ms"),
        end_ms: row.get("end_ms"),
        created_at: row.get("created_at"),
        processing_ms: row.get("processing_ms"),
        translations: HashMap::new(),
    }
}

/// Fill each message's `translations` map from `(message_id, lang, text)` rows.
fn attach_translations(messages: &mut [Message], rows: Vec<(String, String, String)>) {
    let mut by_id: HashMap<String, HashMap<String, String>> = HashMap::new();
    for (mid, lang, text) in rows {
        by_id.entry(mid).or_default().insert(lang, text);
    }
    for m in messages.iter_mut() {
        if let Some(map) = by_id.remove(&m.id) {
            m.translations = map;
        }
    }
}

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
        // Idempotent column migrations for older DBs. The expected "duplicate
        // column name" error on already-migrated databases is ignored; ANY OTHER
        // failure (locked DB, disk full, …) is logged rather than silently
        // swallowed, so a half-applied schema is visible instead of surfacing as
        // mysterious runtime errors later.
        self.add_column("ALTER TABLE conversation ADD COLUMN speaker_names TEXT").await;
        // Multi-language column list (§10.7 N-language mode).
        self.add_column("ALTER TABLE conversation ADD COLUMN langs TEXT").await;
        // Secondary translation for foreign-language rows (§10.7 variant A).
        self.add_column("ALTER TABLE message ADD COLUMN translated_text_b TEXT").await;
        // Speech→text pipeline latency in ms (§10.8).
        self.add_column("ALTER TABLE message ADD COLUMN processing_ms INTEGER").await;
        Ok(())
    }

    /// Run an idempotent `ALTER TABLE ... ADD COLUMN`, ignoring the duplicate-
    /// column error but logging anything unexpected.
    async fn add_column(&self, sql: &str) {
        if let Err(e) = sqlx::query(sql).execute(&self.pool).await {
            if !e.to_string().to_lowercase().contains("duplicate column") {
                log::warn!("migration `{sql}` failed: {e}");
            }
        }
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
        let sql = format!("SELECT {CONV_COLS} FROM conversation ORDER BY updated_at DESC");
        let rows = sqlx::query(&sql).fetch_all(&self.pool).await?;
        Ok(rows.iter().map(row_to_conversation).collect())
    }

    pub async fn get_conversation(&self, id: &str) -> Result<Option<Conversation>, sqlx::Error> {
        let sql = format!("SELECT {CONV_COLS} FROM conversation WHERE id = ?");
        let row = sqlx::query(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(row_to_conversation))
    }

    /// Fetch messages (one conversation or all) and hydrate each one's
    /// per-language `translations` map from `message_translation`.
    async fn fetch_messages(
        &self,
        conversation_id: Option<&str>,
    ) -> Result<Vec<Message>, sqlx::Error> {
        let (msg_sql, tr_sql) = match conversation_id {
            Some(_) => (
                format!(
                    // rowid tie-break: split-segment parts can persist within the same
                    // millisecond; insert order must survive a reload.
                    "SELECT {MSG_COLS} FROM message WHERE conversation_id = ? ORDER BY created_at ASC, rowid ASC"
                ),
                "SELECT mt.message_id, mt.lang, mt.text FROM message_translation mt \
                 JOIN message m ON m.id = mt.message_id WHERE m.conversation_id = ?"
                    .to_string(),
            ),
            None => (
                format!("SELECT {MSG_COLS} FROM message ORDER BY created_at ASC, rowid ASC"),
                "SELECT message_id, lang, text FROM message_translation".to_string(),
            ),
        };
        let mut msg_query = sqlx::query(&msg_sql);
        let mut tr_query = sqlx::query_as::<_, (String, String, String)>(&tr_sql);
        if let Some(id) = conversation_id {
            msg_query = msg_query.bind(id);
            tr_query = tr_query.bind(id);
        }
        let mut messages: Vec<Message> = msg_query
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(row_to_message)
            .collect();
        let rows = tr_query.fetch_all(&self.pool).await?;
        attach_translations(&mut messages, rows);
        Ok(messages)
    }

    pub async fn list_messages(&self, conversation_id: &str) -> Result<Vec<Message>, sqlx::Error> {
        self.fetch_messages(Some(conversation_id)).await
    }

    async fn list_all_messages(&self) -> Result<Vec<Message>, sqlx::Error> {
        self.fetch_messages(None).await
    }

    /// The last `limit` (original_text, translated_text) pairs of a conversation
    /// in chronological order. Purpose-built for the translator's rolling context
    /// (recording.rs::recent_context), which runs per utterance — unlike
    /// [`Self::list_messages`] it doesn't scan the whole conversation or touch
    /// `message_translation`.
    pub async fn recent_message_texts(
        &self,
        conversation_id: &str,
        limit: i64,
    ) -> Result<Vec<(String, String)>, sqlx::Error> {
        let mut rows: Vec<(String, String)> = sqlx::query_as(
            // rowid DESC tie-break: same-millisecond rows come back in exact
            // reverse insert order, so the reverse() below restores it.
            "SELECT original_text, translated_text FROM message \
             WHERE conversation_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?",
        )
        .bind(conversation_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.reverse();
        Ok(rows)
    }

    pub async fn create_conversation(&self, c: &Conversation) -> Result<(), sqlx::Error> {
        let langs_json = (!c.langs.is_empty())
            .then(|| serde_json::to_string(&c.langs).ok())
            .flatten();
        sqlx::query(
            "INSERT INTO conversation (id, title, lang_a, lang_b, langs, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.id)
        .bind(&c.title)
        .bind(&c.lang_a)
        .bind(&c.lang_b)
        .bind(langs_json)
        .bind(c.created_at)
        .bind(c.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Set the ordered language list (§10.7). The first two are mirrored into
    /// `lang_a`/`lang_b` so 2-language back-compat paths keep working.
    pub async fn set_languages(&self, id: &str, langs: &[String], updated_at: i64) -> Result<(), sqlx::Error> {
        let lang_a = langs.first().cloned().unwrap_or_default();
        let lang_b = langs.get(1).cloned().unwrap_or_default();
        let langs_json = serde_json::to_string(langs).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "UPDATE conversation SET langs = ?, lang_a = ?, lang_b = ?, updated_at = ? WHERE id = ?",
        )
        .bind(langs_json)
        .bind(lang_a)
        .bind(lang_b)
        .bind(updated_at)
        .bind(id)
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
        // Keep the `langs` array in sync with the 2-language selector so the
        // multi-language grid never sees a stale list.
        let langs_json = serde_json::to_string(&[lang_a, lang_b]).unwrap_or_else(|_| "[]".into());
        sqlx::query("UPDATE conversation SET lang_a = ?, lang_b = ?, langs = ?, updated_at = ? WHERE id = ?")
            .bind(lang_a)
            .bind(lang_b)
            .bind(langs_json)
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

    pub async fn delete_conversation(&self, id: &str) -> Result<(), sqlx::Error> {
        // ON DELETE CASCADE (foreign_keys=ON) removes messages/summary/notes.
        sqlx::query("DELETE FROM conversation WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_message(&self, m: &Message) -> Result<(), sqlx::Error> {
        // One transaction for the whole upsert: the message row, the destructive
        // DELETE+reINSERT of its per-language translations, and the conversation
        // timestamp land atomically, so a concurrent writer or crash can never
        // observe a message with partially-replaced translations.
        let mut tx = self.pool.begin().await?;
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
        .execute(&mut *tx)
        .await?;
        // Replace the per-language translations (§10.7). A message has at most a
        // few langs; the empty map (2-language path) leaves nothing here.
        sqlx::query("DELETE FROM message_translation WHERE message_id = ?")
            .bind(&m.id)
            .execute(&mut *tx)
            .await?;
        for (lang, text) in &m.translations {
            sqlx::query(
                "INSERT INTO message_translation (message_id, lang, text) VALUES (?, ?, ?)",
            )
            .bind(&m.id)
            .bind(lang)
            .bind(text)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("UPDATE conversation SET updated_at = ? WHERE id = ?")
            .bind(m.created_at)
            .bind(&m.conversation_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
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
            langs: vec!["ru".into(), "en".into()],
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
            translations: std::collections::HashMap::new(),
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
            assert_eq!(boot.conversations[0].langs, vec!["ja", "ko"]);

            // N-language mode (§10.7): per-message translations + langs list.
            let mut m2 = msg("m2", "c1", 250);
            m2.translations.insert("ja".into(), "こんにちは".into());
            m2.translations.insert("en".into(), "Hi".into());
            db.add_message(&m2).await.unwrap();
            let listed = db.list_messages("c1").await.unwrap();
            let got = listed.iter().find(|x| x.id == "m2").unwrap();
            assert_eq!(got.translations.get("ja").map(String::as_str), Some("こんにちは"));
            assert_eq!(got.translations.get("en").map(String::as_str), Some("Hi"));

            db.set_languages("c1", &["ru".into(), "ja".into(), "en".into()], 500)
                .await
                .unwrap();
            let boot = db.bootstrap().await.unwrap();
            assert_eq!(boot.conversations[0].langs, vec!["ru", "ja", "en"]);

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
