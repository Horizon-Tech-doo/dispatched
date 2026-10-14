//! SQLite storage for profiles, sign-ins and settings. Secrets never go here.

use super::{Profile, ProfileAuth, secret_key};
use crate::connstr::ConnectionString;
use crate::namespace::Transport;
use crate::{Error, Result};
use horizon_azure_kit::auth::SignIn;
use horizon_azure_kit::secrets::CredentialStore;
use rusqlite::{Connection, OptionalExtension, params};
use secrecy::SecretString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SCHEMA_VERSION: i64 = 1;

/// Profile database.
#[derive(Debug)]
pub struct ProfileStore {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl ProfileStore {
    /// Open (creating if needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| db_err(path, e))?;
        }
        let conn = Connection::open(path).map_err(|e| db_err(path, e))?;
        let store = Self {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
        };
        store.migrate()?;
        Ok(store)
    }

    /// In-memory database (tests).
    pub fn open_in_memory() -> Result<Self> {
        let path = PathBuf::from(":memory:");
        let conn = Connection::open_in_memory().map_err(|e| db_err(&path, e))?;
        let store = Self {
            conn: Mutex::new(conn),
            path,
        };
        store.migrate()?;
        Ok(store)
    }

    fn err(&self, e: impl std::fmt::Display) -> Error {
        db_err(&self.path, e)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| self.err(e))?;
        if version > SCHEMA_VERSION {
            return Err(self.err(format!(
                "schema version {version} is newer than this Dispatched supports"
            )));
        }
        if version < 1 {
            conn.execute_batch(
                "CREATE TABLE profiles (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
                    host TEXT NOT NULL,
                    auth TEXT NOT NULL,
                    emulator INTEGER NOT NULL DEFAULT 0,
                    production INTEGER NOT NULL DEFAULT 0,
                    color TEXT,
                    transport TEXT NOT NULL DEFAULT 'amqp',
                    proxy TEXT,
                    pinned_entities TEXT NOT NULL DEFAULT '[]'
                );
                CREATE TABLE sign_ins (id TEXT PRIMARY KEY, record TEXT NOT NULL);
                CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                PRAGMA user_version = 1;",
            )
            .map_err(|e| self.err(e))?;
        }
        Ok(())
    }

    /// All profiles, by name.
    pub fn list(&self) -> Result<Vec<Profile>> {
        let conn = self.conn.lock().expect("profile db poisoned");
        let mut stmt = conn
            .prepare(&format!("{SELECT_PROFILE} ORDER BY name COLLATE NOCASE"))
            .map_err(|e| self.err(e))?;
        let rows = stmt
            .query_map([], row_to_profile)
            .map_err(|e| self.err(e))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| self.err(e))
    }

    /// Profile by name (case-insensitive).
    pub fn get(&self, name: &str) -> Result<Option<Profile>> {
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.query_row(
            &format!("{SELECT_PROFILE} WHERE name = ?1 COLLATE NOCASE"),
            params![name],
            row_to_profile,
        )
        .optional()
        .map_err(|e| self.err(e))
    }

    /// Insert a new profile; the name must be unused (case-insensitive).
    pub fn insert(&self, p: &Profile) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        let result = conn.execute(
            "INSERT INTO profiles (id, name, host, auth, emulator, production, color, transport, proxy, pinned_entities)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                p.id, p.name, p.host, auth_json(&p.auth), p.emulator, p.production, p.color,
                transport_text(p.transport), p.proxy, serde_json::to_string(&p.pinned_entities).expect("pins serialize")
            ],
        );
        match result {
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(Error::Invalid(format!(
                    "a profile named `{}` already exists",
                    p.name
                )))
            }
            Err(e) => Err(self.err(e)),
            Ok(_) => Ok(()),
        }
    }

    /// Replace an existing profile (matched by id).
    pub fn update(&self, p: &Profile) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        let result = conn.execute(
            "UPDATE profiles SET name = ?2, host = ?3, auth = ?4, emulator = ?5, production = ?6, color = ?7,
             transport = ?8, proxy = ?9, pinned_entities = ?10 WHERE id = ?1",
            params![
                p.id, p.name, p.host, auth_json(&p.auth), p.emulator, p.production, p.color,
                transport_text(p.transport), p.proxy, serde_json::to_string(&p.pinned_entities).expect("pins serialize")
            ],
        );
        match result {
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(Error::Invalid(format!(
                    "a profile named `{}` already exists",
                    p.name
                )))
            }
            Err(e) => Err(self.err(e)),
            Ok(0) => Err(Error::NotFound(format!("profile `{}`", p.name))),
            Ok(_) => Ok(()),
        }
    }

    /// Validate `cs_text`, store a connection-string profile and its secret.
    pub fn add_connection_string(
        &self,
        name: &str,
        cs_text: &str,
        secrets: &dyn CredentialStore,
        production: bool,
    ) -> Result<Profile> {
        let cs = ConnectionString::parse(cs_text)?;
        let mut profile = Profile::connection_string(name, &cs);
        profile.production = production;
        self.insert(&profile)?;
        if let Err(e) = secrets.put(
            &secret_key(&profile.id),
            &SecretString::from(cs_text.to_string()),
        ) {
            let conn = self.conn.lock().expect("profile db poisoned");
            let _ = conn.execute("DELETE FROM profiles WHERE id = ?1", params![profile.id]);
            return Err(e.into());
        }
        Ok(profile)
    }

    /// Delete a profile and its secret. Returns whether it existed.
    pub fn delete(&self, name: &str, secrets: &dyn CredentialStore) -> Result<bool> {
        let Some(p) = self.get(name)? else {
            return Ok(false);
        };
        secrets.delete(&secret_key(&p.id))?;
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.execute("DELETE FROM profiles WHERE id = ?1", params![p.id])
            .map_err(|e| self.err(e))?;
        let default: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'default_profile'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| self.err(e))?;
        if default.as_deref() == Some(p.id.as_str()) {
            conn.execute("DELETE FROM settings WHERE key = 'default_profile'", [])
                .map_err(|e| self.err(e))?;
        }
        Ok(true)
    }

    fn set_setting(&self, key: &str, value: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        match value {
            Some(v) => conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, v],
            ),
            None => conn.execute("DELETE FROM settings WHERE key = ?1", params![key]),
        }
        .map_err(|e| self.err(e))?;
        Ok(())
    }

    fn setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| self.err(e))
    }

    /// Set (or clear) the default profile.
    pub fn set_default(&self, name: Option<&str>) -> Result<()> {
        match name {
            None => self.set_setting("default_profile", None),
            Some(n) => {
                let p = self
                    .get(n)?
                    .ok_or_else(|| Error::NotFound(format!("profile `{n}`")))?;
                self.set_setting("default_profile", Some(&p.id))
            }
        }
    }

    /// The default profile, if set.
    pub fn default_profile(&self) -> Result<Option<Profile>> {
        let Some(id) = self.setting("default_profile")? else {
            return Ok(None);
        };
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.query_row(
            &format!("{SELECT_PROFILE} WHERE id = ?1"),
            params![id],
            row_to_profile,
        )
        .optional()
        .map_err(|e| self.err(e))
    }

    /// Remember a sign-in (no secrets).
    pub fn save_sign_in(&self, s: &SignIn) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.execute(
            "INSERT INTO sign_ins (id, record) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET record = excluded.record",
            params![s.id, serde_json::to_string(s).expect("SignIn serializes")],
        )
        .map_err(|e| self.err(e))?;
        Ok(())
    }

    /// A remembered sign-in.
    pub fn sign_in(&self, id: &str) -> Result<Option<SignIn>> {
        let conn = self.conn.lock().expect("profile db poisoned");
        let record: Option<String> = conn
            .query_row(
                "SELECT record FROM sign_ins WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| self.err(e))?;
        record
            .map(|r| serde_json::from_str(&r).map_err(|e| self.err(e)))
            .transpose()
    }

    /// Forget a sign-in record.
    pub fn remove_sign_in(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.execute("DELETE FROM sign_ins WHERE id = ?1", params![id])
            .map_err(|e| self.err(e))?;
        Ok(())
    }

    /// Set (or clear) the current sign-in.
    pub fn set_current_sign_in(&self, id: Option<&str>) -> Result<()> {
        self.set_setting("current_sign_in", id)
    }

    /// The current sign-in, if any.
    pub fn current_sign_in(&self) -> Result<Option<SignIn>> {
        match self.setting("current_sign_in")? {
            Some(id) => self.sign_in(&id),
            None => Ok(None),
        }
    }

    /// Import the M0 `cli-state.json` (current sign-in), then delete it.
    /// The database is written first (in one transaction) and the file is
    /// removed last, so a failure part-way loses nothing. A file that cannot
    /// be read or understood is left in place. Returns whether a sign-in was
    /// migrated.
    pub fn migrate_cli_state(&self, path: &Path) -> Result<bool> {
        #[derive(serde::Deserialize)]
        struct OldState {
            sign_in: Option<SignIn>,
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(false);
        };
        let Ok(old) = serde_json::from_str::<OldState>(&text) else {
            return Ok(false);
        };
        if let Some(s) = &old.sign_in {
            let mut conn = self.conn.lock().expect("profile db poisoned");
            let tx = conn.transaction().map_err(|e| self.err(e))?;
            tx.execute(
                "INSERT INTO sign_ins (id, record) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET record = excluded.record",
                params![s.id, serde_json::to_string(s).expect("SignIn serializes")],
            )
            .map_err(|e| self.err(e))?;
            tx.execute(
                "INSERT INTO settings (key, value) VALUES ('current_sign_in', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![s.id],
            )
            .map_err(|e| self.err(e))?;
            tx.commit().map_err(|e| self.err(e))?;
        }
        std::fs::remove_file(path).map_err(|e| {
            Error::Io(format!(
                "could not remove {}: {e}; delete it manually",
                path.display()
            ))
        })?;
        Ok(old.sign_in.is_some())
    }
}

const SELECT_PROFILE: &str = "SELECT id, name, host, auth, emulator, production, color, transport, proxy, pinned_entities FROM profiles";

fn row_to_profile(r: &rusqlite::Row<'_>) -> rusqlite::Result<Profile> {
    let auth_text: String = r.get(3)?;
    let transport_text: String = r.get(7)?;
    let pins_text: String = r.get(9)?;
    Ok(Profile {
        id: r.get(0)?,
        name: r.get(1)?,
        host: r.get(2)?,
        auth: serde_json::from_str(&auth_text).unwrap_or(ProfileAuth::ConnectionString),
        emulator: r.get(4)?,
        production: r.get(5)?,
        color: r.get(6)?,
        transport: Transport::parse(&transport_text).unwrap_or_default(),
        proxy: r.get(8)?,
        pinned_entities: serde_json::from_str(&pins_text).unwrap_or_default(),
    })
}

fn auth_json(a: &ProfileAuth) -> String {
    serde_json::to_string(a).expect("ProfileAuth serializes")
}

fn transport_text(t: Transport) -> &'static str {
    match t {
        Transport::Amqp => "amqp",
        Transport::WebSockets => "websockets",
    }
}

fn db_err(path: &Path, e: impl std::fmt::Display) -> Error {
    Error::Io(format!(
        "profile database error at {}: {e} (delete the file to start over)",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_azure_kit::secrets::MemoryStore;
    use secrecy::ExposeSecret;

    const CS: &str = "Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=Root;SharedAccessKey=TopSecretKey123=";

    #[test]
    fn names_are_case_insensitive_unique() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        store
            .add_connection_string("Prod", CS, &secrets, true)
            .unwrap();
        let e = store
            .add_connection_string("prod", CS, &secrets, false)
            .unwrap_err();
        assert!(e.to_string().contains("already exists"), "{e}");
        assert_eq!(store.get("PROD").unwrap().unwrap().name, "Prod");
    }

    #[test]
    fn secret_never_written_to_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dispatched.db");
        let secrets = MemoryStore::new();
        let p = {
            let store = ProfileStore::open(&path).unwrap();
            store
                .add_connection_string("dev", CS, &secrets, false)
                .unwrap()
        };
        let bytes = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("TopSecretKey123"));
        assert_eq!(
            secrets
                .get(&secret_key(&p.id))
                .unwrap()
                .unwrap()
                .expose_secret(),
            CS
        );
    }

    #[test]
    fn delete_removes_secret() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let p = store
            .add_connection_string("dev", CS, &secrets, false)
            .unwrap();
        assert!(store.delete("DEV", &secrets).unwrap());
        assert!(secrets.get(&secret_key(&p.id)).unwrap().is_none());
        assert!(store.get("dev").unwrap().is_none());
        assert!(!store.delete("dev", &secrets).unwrap());
    }

    #[test]
    fn invalid_connection_string_is_not_stored() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        assert!(
            store
                .add_connection_string("bad", "Endpoint=nope", &secrets, false)
                .is_err()
        );
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn default_profile_and_pins_roundtrip() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let mut p = store
            .add_connection_string("dev", CS, &secrets, false)
            .unwrap();
        p.pinned_entities = vec!["orders".into()];
        p.transport = Transport::WebSockets;
        store.update(&p).unwrap();
        store.set_default(Some("dev")).unwrap();
        let d = store.default_profile().unwrap().unwrap();
        assert_eq!(d.pinned_entities, ["orders"]);
        assert_eq!(d.transport, Transport::WebSockets);
        store.set_default(None).unwrap();
        assert!(store.default_profile().unwrap().is_none());
    }

    #[test]
    fn migrates_cli_state_once() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("cli-state.json");
        std::fs::write(&state, r#"{"sign_in":{"id":"oid-1","user_principal":"a@x.io","tenant_id":"t","display_name":null,"cloud":"public"}}"#).unwrap();
        let store = ProfileStore::open_in_memory().unwrap();
        assert!(store.migrate_cli_state(&state).unwrap());
        assert!(!state.exists(), "old state file is removed");
        assert_eq!(
            store.current_sign_in().unwrap().unwrap().user_principal,
            "a@x.io"
        );
        assert!(!store.migrate_cli_state(&state).unwrap());
    }

    #[test]
    fn corrupt_db_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dispatched.db");
        std::fs::write(&path, b"this is not sqlite").unwrap();
        let e = ProfileStore::open(&path).unwrap_err().to_string();
        assert!(
            e.contains("profile database error") && e.contains("delete the file"),
            "{e}"
        );
    }

    #[test]
    fn unreadable_cli_state_is_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("cli-state.json");
        std::fs::write(&state, "{ not json").unwrap();
        let store = ProfileStore::open_in_memory().unwrap();
        assert!(!store.migrate_cli_state(&state).unwrap());
        assert!(state.exists(), "an unparseable file must not be deleted");
    }

    #[test]
    fn update_of_unknown_profile_is_not_found() {
        let store = ProfileStore::open_in_memory().unwrap();
        let cs = ConnectionString::parse(CS).unwrap();
        let e = store
            .update(&Profile::connection_string("ghost", &cs))
            .unwrap_err();
        assert!(matches!(e, Error::NotFound(_)), "{e}");
    }
}
