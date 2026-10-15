//! SQLite storage for profiles, sign-ins and settings. Secrets never go here.

use super::{DraftAuth, Profile, ProfileAuth, ProfileDraft, namespace_host, secret_key};
use crate::connstr::ConnectionString;
use crate::namespace::{EntityPath, ProxyConfig, Transport};
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

    /// The profile with this id.
    pub fn get_by_id(&self, id: &str) -> Result<Option<Profile>> {
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.query_row(
            &format!("{SELECT_PROFILE} WHERE id = ?1"),
            params![id],
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

    /// Create or edit a profile from a UI draft (see [`ProfileDraft`]).
    pub fn save_draft(
        &self,
        draft: &ProfileDraft,
        secrets: &dyn CredentialStore,
        current_sign_in: Option<&SignIn>,
    ) -> Result<Profile> {
        let pins = clean_pins(&draft.pinned_entities)?;
        match &draft.id {
            None => self.create_from_draft(draft, pins, secrets, current_sign_in),
            Some(id) => self.edit_from_draft(id, draft, pins, secrets),
        }
    }

    fn create_from_draft(
        &self,
        draft: &ProfileDraft,
        pins: Vec<String>,
        secrets: &dyn CredentialStore,
        current_sign_in: Option<&SignIn>,
    ) -> Result<Profile> {
        match &draft.auth {
            DraftAuth::ConnectionString { connection_string } => {
                let text = non_blank(connection_string.as_deref()).ok_or_else(|| {
                    Error::Invalid("paste the namespace's connection string".into())
                })?;
                let cs = ConnectionString::parse(text)?;
                let mut p = Profile::connection_string(&draft.name, &cs);
                let from_cs = std::mem::take(&mut p.pinned_entities);
                apply_draft_options(&mut p, draft, merge_pins(from_cs, pins));
                check_emulator_options(&p)?;
                self.insert(&p)?;
                if let Err(e) =
                    secrets.put(&secret_key(&p.id), &SecretString::from(text.to_string()))
                {
                    self.delete_row(&p.id);
                    return Err(e.into());
                }
                Ok(p)
            }
            DraftAuth::Entra { namespace, tenant } => {
                let sign_in = current_sign_in.ok_or_else(|| {
                    Error::NotSignedIn("sign in with Entra ID first, then add the profile".into())
                })?;
                let host = namespace_host(namespace)?;
                let mut p = Profile::entra(
                    &draft.name,
                    &host,
                    &sign_in.id,
                    non_blank(tenant.as_deref()),
                );
                apply_draft_options(&mut p, draft, pins);
                self.save_sign_in(sign_in)?;
                self.insert(&p)?;
                Ok(p)
            }
        }
    }

    fn edit_from_draft(
        &self,
        id: &str,
        draft: &ProfileDraft,
        pins: Vec<String>,
        secrets: &dyn CredentialStore,
    ) -> Result<Profile> {
        let mut p = self
            .get_by_id(id)?
            .ok_or_else(|| Error::NotFound(format!("profile {id}")))?;
        match (&draft.auth, &p.auth) {
            (DraftAuth::ConnectionString { connection_string }, ProfileAuth::ConnectionString) => {
                if let Some(text) = non_blank(connection_string.as_deref()) {
                    let cs = ConnectionString::parse(text)?;
                    p.host = cs.host.clone();
                    p.emulator = cs.use_development_emulator;
                    apply_draft_options(
                        &mut p,
                        draft,
                        merge_pins(cs.entity_path.into_iter().collect(), pins),
                    );
                    check_emulator_options(&p)?;
                    let key = secret_key(&p.id);
                    let previous = secrets.get(&key)?;
                    secrets.put(&key, &SecretString::from(text.to_string()))?;
                    if let Err(e) = self.update(&p) {
                        let _ = match previous {
                            Some(old) => secrets.put(&key, &old),
                            None => secrets.delete(&key),
                        };
                        return Err(e);
                    }
                    return Ok(p);
                }
            }
            (DraftAuth::Entra { namespace, tenant }, ProfileAuth::Entra { sign_in_id, .. }) => {
                p.host = namespace_host(namespace)?;
                p.auth = ProfileAuth::Entra {
                    sign_in_id: sign_in_id.clone(),
                    tenant: non_blank(tenant.as_deref()).map(str::to_string),
                };
            }
            _ => {
                return Err(Error::Invalid(
                    "a profile's sign-in method can't be changed; add a new profile instead".into(),
                ));
            }
        }
        apply_draft_options(&mut p, draft, pins);
        check_emulator_options(&p)?;
        self.update(&p)?;
        Ok(p)
    }

    /// Remove a profile row only (rollback helper; the secret is handled by the caller).
    fn delete_row(&self, id: &str) {
        let conn = self.conn.lock().expect("profile db poisoned");
        let _ = conn.execute("DELETE FROM profiles WHERE id = ?1", params![id]);
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

    /// An app setting (desktop UI state). App keys live under `app.` so they
    /// can never read or overwrite the store's own keys.
    pub fn app_setting(&self, key: &str) -> Result<Option<String>> {
        self.setting(&app_key(key)?)
    }

    /// Set or clear (`None`) an app setting.
    pub fn set_app_setting(&self, key: &str, value: Option<&str>) -> Result<()> {
        self.set_setting(&app_key(key)?, value)
    }

    /// Remove every app setting whose key starts with `prefix`. Returns how many.
    pub fn clear_app_settings_with_prefix(&self, prefix: &str) -> Result<usize> {
        let full = app_key(prefix)?;
        let conn = self.conn.lock().expect("profile db poisoned");
        // substr instead of LIKE: `_` and `%` in keys must match literally.
        conn.execute(
            "DELETE FROM settings WHERE substr(key, 1, ?2) = ?1",
            params![full, full.chars().count() as i64],
        )
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

    /// Every remembered sign-in whose own tenant is `tenant_id` (ids compare without regard to case),
    /// ordered by sign-in id. A guest-tenant sign-in is its own record, separate from the home one.
    /// The caller picks the one whose refresh token still exists.
    pub fn sign_ins_for_tenant(&self, tenant_id: &str) -> Result<Vec<SignIn>> {
        let conn = self.conn.lock().expect("profile db poisoned");
        let mut stmt = conn
            .prepare("SELECT record FROM sign_ins ORDER BY id")
            .map_err(|e| self.err(e))?;
        let records = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| self.err(e))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| self.err(e))?;
        let mut found = Vec::new();
        for record in records {
            let s: SignIn = serde_json::from_str(&record).map_err(|e| self.err(e))?;
            if s.tenant_id.eq_ignore_ascii_case(tenant_id) {
                found.push(s);
            }
        }
        Ok(found)
    }

    /// The first remembered sign-in for `tenant_id`, if any (see [`Self::sign_ins_for_tenant`]).
    pub fn sign_in_for_tenant(&self, tenant_id: &str) -> Result<Option<SignIn>> {
        Ok(self.sign_ins_for_tenant(tenant_id)?.into_iter().next())
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

fn non_blank(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

fn clean_pins(raw: &[String]) -> Result<Vec<String>> {
    raw.iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| EntityPath::parse(p).map(|_| p.to_string()))
        .collect()
}

fn merge_pins(first: Vec<String>, rest: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in first.into_iter().chain(rest) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&p)) {
            out.push(p);
        }
    }
    out
}

fn apply_draft_options(p: &mut Profile, d: &ProfileDraft, pins: Vec<String>) {
    p.name = d.name.trim().to_string();
    p.production = d.production;
    p.color = non_blank(d.color.as_deref()).map(str::to_string);
    p.transport = d.transport;
    p.proxy = non_blank(d.proxy.as_deref()).map(str::to_string);
    p.pinned_entities = pins;
}

fn check_emulator_options(p: &Profile) -> Result<()> {
    if p.emulator && (p.transport == Transport::WebSockets || p.proxy.is_some()) {
        return Err(Error::Unsupported {
            capability: "WebSockets and proxies on the Service Bus Emulator".into(),
        });
    }
    Ok(())
}

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

const APP_KEY_PREFIX: &str = "app.";
const APP_KEY_MAX: usize = 200;

fn app_key(key: &str) -> Result<String> {
    let ok = !key.is_empty()
        && key.len() <= APP_KEY_MAX
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'));
    if !ok {
        return Err(Error::Invalid(format!(
            "invalid setting key (use 1-{APP_KEY_MAX} letters, digits, `.`, `_`, `:` or `-`)"
        )));
    }
    Ok(format!("{APP_KEY_PREFIX}{key}"))
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
    fn sign_ins_are_found_by_their_own_tenant() {
        let store = ProfileStore::open_in_memory().unwrap();
        let mut home = sign_in("home-oid");
        home.tenant_id = "tenant-home".into();
        let mut guest = sign_in("guest-oid");
        guest.tenant_id = "TENANT-GUEST".into();
        let mut guest2 = sign_in("a-guest-oid");
        guest2.tenant_id = "tenant-guest".into();
        for s in [&home, &guest, &guest2] {
            store.save_sign_in(s).unwrap();
        }
        assert_eq!(
            store.sign_in_for_tenant("tenant-home").unwrap().unwrap().id,
            "home-oid"
        );
        // Case-insensitive, and every match comes back in id order.
        let ids: Vec<_> = store
            .sign_ins_for_tenant("Tenant-Guest")
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, ["a-guest-oid", "guest-oid"]);
        assert_eq!(
            store
                .sign_in_for_tenant("tenant-guest")
                .unwrap()
                .unwrap()
                .id,
            "a-guest-oid"
        );
        assert!(store.sign_in_for_tenant("nobody").unwrap().is_none());
        assert!(store.sign_ins_for_tenant("").unwrap().is_empty());
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

    fn entra_profile(store: &ProfileStore, name: &str) -> Profile {
        let p = Profile::entra(name, "myns.servicebus.windows.net", "sign-in-1", None);
        store.insert(&p).unwrap();
        p
    }

    #[test]
    fn get_by_id_finds_and_misses() {
        let store = ProfileStore::open_in_memory().unwrap();
        let p = entra_profile(&store, "Logistics");
        assert_eq!(store.get_by_id(&p.id).unwrap().unwrap().name, "Logistics");
        assert!(store.get_by_id("nope").unwrap().is_none());
    }

    #[test]
    fn app_settings_round_trip_and_clear() {
        let store = ProfileStore::open_in_memory().unwrap();
        assert!(store.app_setting("ui").unwrap().is_none());
        store
            .set_app_setting("ui", Some(r#"{"theme":"dark"}"#))
            .unwrap();
        assert_eq!(
            store.app_setting("ui").unwrap().as_deref(),
            Some(r#"{"theme":"dark"}"#)
        );
        store.set_app_setting("ui", None).unwrap();
        assert!(store.app_setting("ui").unwrap().is_none());
    }

    #[test]
    fn app_keys_cannot_reach_internal_keys() {
        let store = ProfileStore::open_in_memory().unwrap();
        let p = entra_profile(&store, "Logistics");
        store.set_default(Some("Logistics")).unwrap();
        // The internal key is invisible through the app API...
        assert!(store.app_setting("default_profile").unwrap().is_none());
        // ...and writing the same name through it does not touch the internal one.
        store.set_app_setting("default_profile", Some("x")).unwrap();
        assert_eq!(store.default_profile().unwrap().unwrap().id, p.id);
    }

    #[test]
    fn invalid_app_keys_are_rejected() {
        let store = ProfileStore::open_in_memory().unwrap();
        for bad in ["", "has space", "semi;colon", &"k".repeat(201)] {
            let e = store.set_app_setting(bad, Some("1")).unwrap_err();
            assert!(e.to_string().contains("setting key"), "{bad}: {e}");
        }
    }

    #[test]
    fn clear_app_settings_with_prefix_only_removes_matching() {
        let store = ProfileStore::open_in_memory().unwrap();
        store.set_app_setting("shell:a_1", Some("1")).unwrap();
        store.set_app_setting("shell:a_1.extra", Some("2")).unwrap();
        store.set_app_setting("shell:ab1", Some("3")).unwrap();
        store.set_app_setting("ui", Some("4")).unwrap();
        // `_` must not act as a wildcard.
        assert_eq!(
            store.clear_app_settings_with_prefix("shell:a_1").unwrap(),
            2
        );
        assert!(store.app_setting("shell:a_1").unwrap().is_none());
        assert_eq!(
            store.app_setting("shell:ab1").unwrap().as_deref(),
            Some("3")
        );
        assert_eq!(store.app_setting("ui").unwrap().as_deref(), Some("4"));
    }

    use crate::profiles::{DraftAuth, ProfileDraft};
    use horizon_azure_kit::AzureCloud;
    use horizon_azure_kit::secrets::CredentialStore;

    const DCS: &str = "Endpoint=sb://myns.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=c2VjcmV0LWtleQ==";
    const DCS2: &str = "Endpoint=sb://other.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=bmV3LWtleQ==";
    const EMU: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

    fn cs_draft(name: &str, cs: Option<&str>) -> ProfileDraft {
        ProfileDraft {
            id: None,
            name: name.into(),
            auth: DraftAuth::ConnectionString {
                connection_string: cs.map(str::to_string),
            },
            production: false,
            color: None,
            transport: Transport::Amqp,
            proxy: None,
            pinned_entities: vec![],
        }
    }

    fn draft_sign_in() -> SignIn {
        SignIn {
            id: "oid-1".into(),
            user_principal: "ops@contoso.example".into(),
            tenant_id: "tenant-1".into(),
            display_name: None,
            cloud: AzureCloud::Public,
        }
    }

    fn stored(secrets: &MemoryStore, p: &Profile) -> Option<String> {
        secrets
            .get(&secret_key(&p.id))
            .unwrap()
            .map(|s| s.expose_secret().to_string())
    }

    #[test]
    fn draft_secret_never_reaches_the_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dispatched.db");
        let store = ProfileStore::open(&path).unwrap();
        let secrets = MemoryStore::new();
        let mut d = cs_draft("Logistics", Some(DCS));
        d.production = true;
        d.color = Some("  amber ".into());
        let p = store.save_draft(&d, &secrets, None).unwrap();
        assert_eq!(p.host, "myns.servicebus.windows.net");
        assert!(p.production);
        assert_eq!(p.color.as_deref(), Some("amber"));
        assert_eq!(stored(&secrets, &p).as_deref(), Some(DCS));
        drop(store);
        let bytes = std::fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("c2VjcmV0LWtleQ"), "secret written to the DB");
    }

    #[test]
    fn create_needs_a_connection_string() {
        let store = ProfileStore::open_in_memory().unwrap();
        let e = store
            .save_draft(&cs_draft("x", Some("  ")), &MemoryStore::new(), None)
            .unwrap_err();
        assert!(
            e.to_string()
                .contains("paste the namespace's connection string"),
            "{e}"
        );
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn bad_connection_string_error_does_not_echo_it() {
        let store = ProfileStore::open_in_memory().unwrap();
        let bad = "Endpoint=sb://x/;SharedAccessKey=TOPSECRET;SharedAccessKeyName";
        let e = store
            .save_draft(&cs_draft("x", Some(bad)), &MemoryStore::new(), None)
            .unwrap_err();
        assert!(!e.to_string().contains("TOPSECRET"), "{e}");
    }

    #[test]
    fn edit_without_secret_keeps_it_and_with_secret_replaces_it() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let p = store
            .save_draft(&cs_draft("Logistics", Some(DCS)), &secrets, None)
            .unwrap();

        let mut edit = cs_draft("Logistics prod", None);
        edit.id = Some(p.id.clone());
        let p2 = store.save_draft(&edit, &secrets, None).unwrap();
        assert_eq!(p2.name, "Logistics prod");
        assert_eq!(stored(&secrets, &p2).as_deref(), Some(DCS));

        let mut edit = cs_draft("Logistics prod", Some(DCS2));
        edit.id = Some(p.id.clone());
        let p3 = store.save_draft(&edit, &secrets, None).unwrap();
        assert_eq!(p3.host, "other.servicebus.windows.net");
        assert_eq!(stored(&secrets, &p3).as_deref(), Some(DCS2));
    }

    #[test]
    fn rename_conflict_restores_the_previous_secret() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        store
            .save_draft(&cs_draft("A", Some(DCS)), &secrets, None)
            .unwrap();
        let b = store
            .save_draft(&cs_draft("B", Some(DCS)), &secrets, None)
            .unwrap();
        let mut edit = cs_draft("a", Some(DCS2));
        edit.id = Some(b.id.clone());
        let e = store.save_draft(&edit, &secrets, None).unwrap_err();
        assert!(e.to_string().contains("already exists"), "{e}");
        assert_eq!(stored(&secrets, &b).as_deref(), Some(DCS));
        assert_eq!(
            store.get_by_id(&b.id).unwrap().unwrap().host,
            "myns.servicebus.windows.net"
        );
    }

    #[test]
    fn edit_cannot_switch_auth_kind_or_target_a_missing_profile() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let p = store
            .save_draft(&cs_draft("A", Some(DCS)), &secrets, None)
            .unwrap();
        let mut edit = cs_draft("A", None);
        edit.id = Some(p.id.clone());
        edit.auth = DraftAuth::Entra {
            namespace: "myns".into(),
            tenant: None,
        };
        let e = store
            .save_draft(&edit, &secrets, Some(&draft_sign_in()))
            .unwrap_err();
        assert!(e.to_string().contains("can't be changed"), "{e}");
        edit.id = Some("missing".into());
        assert!(matches!(
            store
                .save_draft(&edit, &secrets, Some(&draft_sign_in()))
                .unwrap_err(),
            Error::NotFound(_)
        ));
    }

    #[test]
    fn entra_draft_needs_a_sign_in_and_expands_the_host() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let mut d = cs_draft("Ops", None);
        d.auth = DraftAuth::Entra {
            namespace: "MyNs".into(),
            tenant: Some(" t-2 ".into()),
        };
        assert!(matches!(
            store.save_draft(&d, &secrets, None).unwrap_err(),
            Error::NotSignedIn(_)
        ));
        let p = store
            .save_draft(&d, &secrets, Some(&draft_sign_in()))
            .unwrap();
        assert_eq!(p.host, "myns.servicebus.windows.net");
        assert_eq!(
            p.auth,
            ProfileAuth::Entra {
                sign_in_id: "oid-1".into(),
                tenant: Some("t-2".into())
            }
        );
        assert!(store.sign_in("oid-1").unwrap().is_some());
    }

    #[test]
    fn emulator_draft_rejects_websockets_and_writes_nothing() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let mut d = cs_draft("Local", Some(EMU));
        d.transport = Transport::WebSockets;
        assert!(matches!(
            store.save_draft(&d, &secrets, None).unwrap_err(),
            Error::Unsupported { .. }
        ));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn pins_are_validated_and_merged_with_the_entity_path() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let mut d = cs_draft("Scoped", Some(&format!("{DCS};EntityPath=orders")));
        d.pinned_entities = vec![
            "ORDERS".into(),
            " ".into(),
            "events/subscriptions/audit".into(),
        ];
        let p = store.save_draft(&d, &secrets, None).unwrap();
        assert_eq!(
            p.pinned_entities,
            vec!["orders", "events/subscriptions/audit"]
        );
        let mut bad = cs_draft("Bad", Some(DCS));
        bad.pinned_entities = vec!["a//b".into()];
        assert!(matches!(
            store.save_draft(&bad, &secrets, None).unwrap_err(),
            Error::Invalid(_)
        ));
    }

    #[test]
    fn draft_debug_redacts_the_connection_string() {
        let d = cs_draft("x", Some(DCS));
        assert!(!format!("{d:?}").contains("c2VjcmV0"));
    }

    #[test]
    fn draft_deserializes_from_ui_json() {
        let d: ProfileDraft = serde_json::from_str(
            r#"{"name":"Ops","auth":{"kind":"entra","namespace":"myns"},"transport":"websockets"}"#,
        )
        .unwrap();
        assert_eq!(d.transport, Transport::WebSockets);
        assert!(d.id.is_none() && !d.production && d.pinned_entities.is_empty());
    }
}
