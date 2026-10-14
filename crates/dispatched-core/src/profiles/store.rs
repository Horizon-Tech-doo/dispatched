//! SQLite storage for profiles, sign-ins and settings. Secrets never go here.

use super::{Profile, ProfileAuth, secret_key};
use crate::connstr::ConnectionString;
use crate::namespace::{ProxyConfig, Transport};
use crate::{Error, Result};
use horizon_azure_kit::auth::SignIn;
use horizon_azure_kit::secrets::CredentialStore;
use rusqlite::{Connection, OptionalExtension, params};
use secrecy::SecretString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SCHEMA_VERSION: i64 = 1;

/// How long a second process waits for the first to finish creating the schema.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Profile database.
#[derive(Debug)]
pub struct ProfileStore {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl ProfileStore {
    /// Open (creating if needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            create_private_dir(dir).map_err(|e| db_err(path, e))?;
        }
        let conn = Connection::open(path).map_err(|e| db_err(path, e))?;
        conn.busy_timeout(BUSY_TIMEOUT)
            .map_err(|e| db_err(path, e))?;
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

    /// Map an error from reading profile rows: a damaged profile says how to
    /// remove it; anything else is a database-level failure.
    fn row_err(&self, e: rusqlite::Error) -> Error {
        if let rusqlite::Error::FromSqlConversionFailure(_, _, inner) = &e
            && let Some(c) = inner.downcast_ref::<CorruptProfile>()
        {
            return Error::Io(c.to_string());
        }
        self.err(e)
    }

    fn migrate(&self) -> Result<()> {
        let mut conn = self.conn.lock().expect("profile db poisoned");
        // Take the write lock first and read the version under it, so a second
        // process that lost the race sees version 1 and skips creation.
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| self.err(e))?;
        let version: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| self.err(e))?;
        if version > SCHEMA_VERSION {
            return Err(self.err(format!(
                "schema version {version} is newer than this Dispatched supports"
            )));
        }
        if version < 1 {
            tx.execute_batch(
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
        tx.commit().map_err(|e| self.err(e))
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
            .map_err(|e| self.row_err(e))
    }

    /// Profile by name (case-insensitive).
    pub fn get(&self, name: &str) -> Result<Option<Profile>> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(None);
        }
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.query_row(
            &format!("{SELECT_PROFILE} WHERE name = ?1 COLLATE NOCASE"),
            params![name],
            row_to_profile,
        )
        .optional()
        .map_err(|e| self.row_err(e))
    }

    /// Insert a new profile; the name must be unused (case-insensitive).
    pub fn insert(&self, p: &Profile) -> Result<()> {
        let name = clean_name(&p.name)?;
        validate_proxy(p.proxy.as_deref())?;
        let conn = self.conn.lock().expect("profile db poisoned");
        let result = conn.execute(
            "INSERT INTO profiles (id, name, host, auth, emulator, production, color, transport, proxy, pinned_entities)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                p.id, name, p.host, auth_json(&p.auth), p.emulator, p.production, p.color,
                transport_text(p.transport), p.proxy, serde_json::to_string(&p.pinned_entities).expect("pins serialize")
            ],
        );
        match result {
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(Error::Invalid(format!(
                    "a profile named `{}` already exists",
                    name
                )))
            }
            Err(e) => Err(self.err(e)),
            Ok(_) => Ok(()),
        }
    }

    /// Replace an existing profile (matched by id).
    pub fn update(&self, p: &Profile) -> Result<()> {
        let name = clean_name(&p.name)?;
        validate_proxy(p.proxy.as_deref())?;
        let conn = self.conn.lock().expect("profile db poisoned");
        let result = conn.execute(
            "UPDATE profiles SET name = ?2, host = ?3, auth = ?4, emulator = ?5, production = ?6, color = ?7,
             transport = ?8, proxy = ?9, pinned_entities = ?10 WHERE id = ?1",
            params![
                p.id, name, p.host, auth_json(&p.auth), p.emulator, p.production, p.color,
                transport_text(p.transport), p.proxy, serde_json::to_string(&p.pinned_entities).expect("pins serialize")
            ],
        );
        match result {
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(Error::Invalid(format!(
                    "a profile named `{}` already exists",
                    name
                )))
            }
            Err(e) => Err(self.err(e)),
            Ok(0) => Err(Error::NotFound(format!("profile `{name}`"))),
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
        let name = name.trim();
        // Only the id is read, so a profile with damaged settings can still be removed.
        let id: Option<String> = {
            let conn = self.conn.lock().expect("profile db poisoned");
            conn.query_row(
                "SELECT id FROM profiles WHERE name = ?1 COLLATE NOCASE",
                params![name],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| self.err(e))?
        };
        let Some(id) = id else { return Ok(false) };
        secrets.delete(&secret_key(&id))?;
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.execute("DELETE FROM profiles WHERE id = ?1", params![id])
            .map_err(|e| self.err(e))?;
        conn.execute(
            "DELETE FROM settings WHERE key = 'default_profile' AND value = ?1",
            params![id],
        )
        .map_err(|e| self.err(e))?;
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
        .map_err(|e| self.row_err(e))
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
            // Only fill an unset choice: a stale file must not override a newer one.
            tx.execute(
                "INSERT INTO settings (key, value) VALUES ('current_sign_in', ?1) ON CONFLICT(key) DO NOTHING",
                params![s.id],
            )
            .map_err(|e| self.err(e))?;
            tx.commit().map_err(|e| self.err(e))?;
        }
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(Error::Io(format!(
                    "could not remove {}: {e}; delete it manually",
                    path.display()
                )));
            }
        }
        Ok(old.sign_in.is_some())
    }
}

const SELECT_PROFILE: &str = "SELECT id, name, host, auth, emulator, production, color, transport, proxy, pinned_entities FROM profiles";

/// A profile row whose stored settings cannot be understood.
#[derive(Debug)]
struct CorruptProfile(String);

impl std::fmt::Display for CorruptProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CorruptProfile {}

fn row_to_profile(r: &rusqlite::Row<'_>) -> rusqlite::Result<Profile> {
    let id: String = r.get(0)?;
    let name: String = r.get(1)?;
    let bad = |col: usize, what: &str| {
        rusqlite::Error::FromSqlConversionFailure(
            col,
            rusqlite::types::Type::Text,
            Box::new(CorruptProfile(format!(
                "profile `{name}` has an invalid saved {what}; remove it with `dispatched profile rm {name}` and add it again"
            ))),
        )
    };
    let auth_text: String = r.get(3)?;
    let transport_text: String = r.get(7)?;
    let pins_text: String = r.get(9)?;
    let auth = serde_json::from_str(&auth_text).map_err(|_| bad(3, "sign-in method"))?;
    let transport = Transport::parse(&transport_text).map_err(|_| bad(7, "transport"))?;
    let pinned_entities =
        serde_json::from_str(&pins_text).map_err(|_| bad(9, "pinned-entities list"))?;
    Ok(Profile {
        host: r.get(2)?,
        auth,
        emulator: r.get(4)?,
        production: r.get(5)?,
        color: r.get(6)?,
        transport,
        proxy: r.get(8)?,
        pinned_entities,
        id,
        name,
    })
}

/// Trim a profile name and refuse an empty one.
fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Invalid(
            "a profile needs a name; give it one that is not blank".into(),
        ));
    }
    Ok(name.to_string())
}

/// Proxies are stored without credentials (they would end up in the
/// database); an authenticated proxy comes from `HTTPS_PROXY`.
fn validate_proxy(proxy: Option<&str>) -> Result<()> {
    let Some(proxy) = proxy else { return Ok(()) };
    ProxyConfig::parse(proxy)?;
    if let Ok(u) = url::Url::parse(proxy)
        && (!u.username().is_empty() || u.password().is_some())
    {
        return Err(Error::Invalid(
            "store the proxy without credentials (http://host:port); set HTTPS_PROXY for an authenticated proxy".into(),
        ));
    }
    Ok(())
}

/// Create `dir` (and missing parents); new directories are private (0700) on Unix.
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
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
        let e = store
            .add_connection_string("  prod ", CS, &secrets, false)
            .unwrap_err();
        assert!(e.to_string().contains("named `prod` already"), "{e}");
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

    struct FailingStore;
    impl CredentialStore for FailingStore {
        fn put(&self, _: &str, _: &SecretString) -> horizon_azure_kit::KitResult<()> {
            Err(horizon_azure_kit::KitError::SecretStore("boom".into()))
        }
        fn get(&self, _: &str) -> horizon_azure_kit::KitResult<Option<SecretString>> {
            Ok(None)
        }
        fn delete(&self, _: &str) -> horizon_azure_kit::KitResult<()> {
            Ok(())
        }
    }

    #[test]
    fn failed_secret_write_leaves_no_row() {
        let store = ProfileStore::open_in_memory().unwrap();
        let e = store
            .add_connection_string("dev", CS, &FailingStore, false)
            .unwrap_err();
        assert!(e.to_string().contains("boom"), "{e}");
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn production_flag_roundtrips() {
        let store = ProfileStore::open_in_memory().unwrap();
        store
            .add_connection_string("prod", CS, &MemoryStore::new(), true)
            .unwrap();
        assert!(store.get("prod").unwrap().unwrap().production);
    }

    #[test]
    fn concurrent_opens_both_succeed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("dispatched.db");
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    s.spawn(|| {
                        barrier.wait();
                        ProfileStore::open(&path).map(|_| ())
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap().unwrap();
            }
        });
    }

    #[cfg(unix)]
    #[test]
    fn new_config_dir_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("dispatched");
        ProfileStore::open(&sub.join("dispatched.db")).unwrap();
        assert_eq!(
            std::fs::metadata(&sub).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    fn sign_in(id: &str) -> SignIn {
        SignIn {
            id: id.into(),
            user_principal: format!("{id}@x.io"),
            tenant_id: "t".into(),
            display_name: None,
            cloud: horizon_azure_kit::AzureCloud::Public,
        }
    }

    #[test]
    fn stale_state_file_does_not_override_current_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("cli-state.json");
        std::fs::write(
            &state,
            serde_json::json!({ "sign_in": sign_in("old") }).to_string(),
        )
        .unwrap();
        let store = ProfileStore::open_in_memory().unwrap();
        store.save_sign_in(&sign_in("new")).unwrap();
        store.set_current_sign_in(Some("new")).unwrap();
        store.migrate_cli_state(&state).unwrap();
        assert_eq!(store.current_sign_in().unwrap().unwrap().id, "new");
        assert!(
            store.sign_in("old").unwrap().is_some(),
            "the old record is still imported"
        );
    }

    #[test]
    fn proxy_credentials_are_rejected() {
        let store = ProfileStore::open_in_memory().unwrap();
        let cs = ConnectionString::parse(CS).unwrap();
        for bad in [
            "http://u:p@proxy:8080",
            "http://u@proxy:8080",
            "http://:p@proxy:8080",
        ] {
            let mut p = Profile::connection_string("dev", &cs);
            p.proxy = Some(bad.into());
            let e = store.insert(&p).unwrap_err().to_string();
            assert!(
                e.contains("without credentials") && e.contains("HTTPS_PROXY"),
                "{e}"
            );
            assert!(!e.contains(":p@"), "{e}");
        }
        let mut p = Profile::connection_string("dev", &cs);
        p.proxy = Some("not a url".into());
        assert!(store.insert(&p).is_err());
        p.proxy = Some("http://proxy:8080".into());
        store.insert(&p).unwrap();
        p.proxy = Some("http://u:p@proxy:8080".into());
        assert!(store.update(&p).is_err());
    }

    #[test]
    fn corrupt_rows_name_the_profile() {
        for (column, value, what) in [
            ("auth", "{nope", "sign-in"),
            ("transport", "carrier-pigeon", "transport"),
            ("pinned_entities", "[1,", "pinned"),
        ] {
            let store = ProfileStore::open_in_memory().unwrap();
            store
                .add_connection_string("dev", CS, &MemoryStore::new(), false)
                .unwrap();
            store
                .conn
                .lock()
                .unwrap()
                .execute(
                    &format!("UPDATE profiles SET {column} = ?1"),
                    params![value],
                )
                .unwrap();
            for e in [store.get("dev").unwrap_err(), store.list().unwrap_err()] {
                let e = e.to_string();
                assert!(
                    e.contains("profile `dev`")
                        && e.contains(what)
                        && e.contains("dispatched profile rm dev")
                        && !e.contains("delete the file"),
                    "{e}"
                );
            }
        }
    }

    #[test]
    fn blank_names_are_rejected_and_names_trimmed() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        for blank in ["", "   ", "\t"] {
            let e = store
                .add_connection_string(blank, CS, &secrets, false)
                .unwrap_err();
            assert!(matches!(e, Error::Invalid(_)), "{e}");
        }
        let mut p = store
            .add_connection_string("  dev  ", CS, &secrets, false)
            .unwrap();
        assert_eq!(p.name, "dev");
        assert_eq!(store.get("  DEV ").unwrap().unwrap().id, p.id);
        assert!(store.get("  ").unwrap().is_none());
        p.name = "  renamed ".into();
        store.update(&p).unwrap();
        assert_eq!(store.get("renamed").unwrap().unwrap().id, p.id);
        p.name = "   ".into();
        assert!(matches!(store.update(&p).unwrap_err(), Error::Invalid(_)));
    }

    #[test]
    fn corrupt_profile_can_still_be_removed() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let dev = store
            .add_connection_string("dev", CS, &secrets, false)
            .unwrap();
        let other = store
            .add_connection_string("other", CS, &secrets, false)
            .unwrap();
        store.set_default(Some("dev")).unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE profiles SET transport = 'carrier-pigeon', auth = '{nope' WHERE name = 'dev'",
                [],
            )
            .unwrap();
        assert!(store.list().is_err());
        assert!(store.delete(" DEV ", &secrets).unwrap());
        assert!(secrets.get(&secret_key(&dev.id)).unwrap().is_none());
        assert!(secrets.get(&secret_key(&other.id)).unwrap().is_some());
        let left = store.list().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].name, "other");
        assert!(store.default_profile().unwrap().is_none());
    }
}
