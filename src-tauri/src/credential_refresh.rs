//! Coordinated OAuth rotation. The registries contain fingerprints/status only;
//! disk credentials and response bodies are never retained in global state.
use crate::private_file::PrivateStage;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CredentialVersion([u8; 32]);
impl std::fmt::Debug for CredentialVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialVersion([redacted])")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommitOutcome {
    Written,
    ChangedExternally,
}
pub(crate) fn credential_version(bytes: &[u8]) -> CredentialVersion {
    CredentialVersion(Sha256::digest(bytes).into())
}
#[derive(Clone)]
pub(crate) enum Flavor {
    Claude { account: Option<String> },
    Codex,
    Kimi,
    Grok { entry: String },
}
#[derive(Clone, Copy)]
pub(crate) enum Reason {
    Expiry,
    Rejected(CredentialVersion),
}
pub(crate) struct RefreshInput {
    pub refresh: String,
    pub doc: Value,
}
pub(crate) enum RemoteError {
    Rejected,
    Unavailable,
    SucceededInvalid,
}
pub(crate) struct FileResult {
    pub doc: Value,
    pub outcome: Option<CommitOutcome>,
}
impl std::fmt::Debug for FileResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileResult([redacted])")
    }
}
const MAX_CREDENTIAL_BYTES: u64 = 64 * 1024;
const UNCERTAIN: &str =
    "refresh outcome uncertain; update this source or sign in again before retrying";
const SAVE_FAILED: &str =
    "refresh succeeded, save failed; update this source or sign in again before retrying";

#[derive(Default)]
pub(crate) struct SourceState {
    blocked: Option<(CredentialVersion, &'static str)>,
    completed: Option<CredentialVersion>,
}
impl SourceState {
    pub(crate) fn check(&self, version: CredentialVersion) -> Result<(), String> {
        if let Some((v, reason)) = self.blocked {
            if v == version {
                return Err(reason.into());
            }
        }
        Ok(())
    }
    pub(crate) fn attempted(&mut self, version: CredentialVersion) {
        self.blocked = Some((version, UNCERTAIN));
    }
    pub(crate) fn save_failed(&mut self, version: CredentialVersion) {
        self.blocked = Some((version, SAVE_FAILED));
    }
    pub(crate) fn completed(&mut self, version: CredentialVersion) {
        self.blocked = None;
        self.completed = Some(version);
    }
}
pub(crate) async fn lock_source(key: &str) -> tokio::sync::OwnedMutexGuard<SourceState> {
    static SOURCES: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<SourceState>>>>> =
        OnceLock::new();
    let lock = SOURCES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key.into())
        .or_default()
        .clone();
    lock.lock_owned().await
}
struct AttemptStamp {
    source: String,
    version: CredentialVersion,
    finished: bool,
    rotated: bool,
    at: std::time::Instant,
}
pub(crate) struct Attempt {
    guard: tokio::sync::OwnedMutexGuard<Option<AttemptStamp>>,
    // Acquiring the token lock is only a reservation. Preserve the prior
    // stamp until the caller has finished all known-local preflight checks.
    reserved: Option<AttemptStamp>,
}
impl Attempt {
    pub(crate) fn started(&mut self) {
        if let Some(stamp) = self.reserved.take() {
            *self.guard = Some(stamp);
        }
    }
    pub(crate) fn finish(&mut self, rotated: bool) {
        if self.reserved.is_some() {
            return;
        }
        if let Some(s) = self.guard.as_mut() {
            s.finished = true;
            s.rotated = rotated;
            s.at = std::time::Instant::now();
        }
    }
}
/// A second path containing the same rotating credential must not redeem it
/// again. No returned credential is shared with another (possibly ungranted)
/// source. Nonrotating tokens become reusable; a short duplicate guard avoids
/// a concurrent copied source immediately repeating the same exchange.
pub(crate) async fn begin_attempt(
    source: &str,
    family: &str,
    refresh: &str,
    version: CredentialVersion,
    check: impl Fn() -> Result<(), String>,
) -> Result<Attempt, String> {
    static TOKENS: OnceLock<
        Mutex<HashMap<CredentialVersion, Arc<tokio::sync::Mutex<Option<AttemptStamp>>>>>,
    > = OnceLock::new();
    let key = pair_version(family, refresh);
    let lock = TOKENS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key)
        .or_default()
        .clone();
    let guard = lock.lock_owned().await;
    check()?;
    if let Some(s) = guard.as_ref() {
        if !s.finished || s.rotated {
            return Err("This refresh credential was already used or its outcome is uncertain; update this source or sign in again".into());
        }
        if !(s.source == source && s.version != version)
            && s.at.elapsed() < std::time::Duration::from_secs(30)
        {
            return Err("Another refresh of this credential just completed; reread this source before retrying".into());
        }
    }
    Ok(Attempt {
        guard,
        reserved: Some(AttemptStamp {
            source: source.into(),
            version,
            finished: false,
            rotated: false,
            at: std::time::Instant::now(),
        }),
    })
}
pub(crate) fn pair_version(access: &str, refresh: &str) -> CredentialVersion {
    let mut hash = Sha256::new();
    hash.update((access.len() as u64).to_le_bytes());
    hash.update(access.as_bytes());
    hash.update((refresh.len() as u64).to_le_bytes());
    hash.update(refresh.as_bytes());
    CredentialVersion(hash.finalize().into())
}
pub(crate) fn version_text(version: CredentialVersion) -> String {
    version.0.iter().map(|b| format!("{b:02x}")).collect()
}

/// Canonicalizes once, rejecting observed symlink/reparse components and hard
/// linked files. Native Windows alias behavior still requires native tests.
fn canonical_without_links(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| "source directory unavailable")?
            .join(path)
    };
    let mut walked = PathBuf::new();
    for component in absolute.components() {
        walked.push(component.as_os_str());
        let m = std::fs::symlink_metadata(&walked).map_err(|_| "credential source unavailable")?;
        if is_link(&m) {
            return Err("credential source contains a link or reparse point".into());
        }
    }
    absolute
        .canonicalize()
        .map_err(|_| "credential source unavailable".into())
}
pub(crate) fn normalized_source(path: &Path) -> Result<PathBuf, String> {
    let canonical = canonical_without_links(path)?;
    let m = std::fs::symlink_metadata(&canonical).map_err(|_| "credential source unavailable")?;
    if !m.is_file() || is_link(&m) {
        return Err("credential source is not a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.nlink() != 1 {
            return Err("hard-linked credential sources are unsupported".into());
        }
    }
    Ok(canonical)
}
fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
pub(crate) fn read_bytes(path: &Path, cap: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let before = std::fs::symlink_metadata(path).map_err(|_| "credential source unavailable")?;
    if !before.is_file() || is_link(&before) {
        return Err("credential source is not a regular file".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open an observed final reparse point itself rather than following it
        // if the pathname changes between metadata and open.
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options
        .open(path)
        .map_err(|_| "credential source unavailable")?;
    let opened = file
        .metadata()
        .map_err(|_| "credential source unavailable")?;
    if !opened.is_file() || is_link(&opened) {
        return Err("credential source is not a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.nlink() != 1 || before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Err("credential source changed while opening".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| "credential file identity unavailable")?;
        if info.nNumberOfLinks != 1 {
            return Err("hard-linked credential sources are unsupported".into());
        }
    }
    let mut out = Vec::new();
    file.take(cap + 1)
        .read_to_end(&mut out)
        .map_err(|_| "credential source read failed")?;
    if out.len() as u64 > cap {
        return Err("credential source exceeds size limit".into());
    }
    Ok(out)
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExpectedVersion {
    Present(CredentialVersion),
    Absent,
}
pub(crate) fn expected_version(path: &Path) -> Result<ExpectedVersion, String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ExpectedVersion::Absent),
        Err(_) => Err("credential source unavailable".into()),
        Ok(_) => Ok(ExpectedVersion::Present(credential_version(&read_bytes(
            path,
            MAX_CREDENTIAL_BYTES,
        )?))),
    }
}
pub(crate) fn commit_stage(
    path: &Path,
    expected: ExpectedVersion,
    mut stage: PrivateStage,
    updated: &[u8],
    check: impl Fn() -> Result<(), String>,
) -> Result<CommitOutcome, String> {
    check()?;
    if updated.len() as u64 > MAX_CREDENTIAL_BYTES {
        return Err("updated credential document exceeds size limit".into());
    }
    stage.write(updated)?;
    check()?;
    if expected_version(path)? != expected {
        return Ok(CommitOutcome::ChangedExternally);
    }
    check()?;
    stage.commit_checked(path, check)?;
    Ok(CommitOutcome::Written)
}
pub(crate) fn commit_if_unchanged(
    path: &Path,
    expected: CredentialVersion,
    updated: &[u8],
) -> Result<CommitOutcome, String> {
    crate::access_policy::check_current_operation()?;
    let stage = PrivateStage::new(path)?;
    commit_stage(
        path,
        ExpectedVersion::Present(expected),
        stage,
        updated,
        crate::access_policy::check_current_operation,
    )
}
fn read_doc(path: &Path) -> Result<(CredentialVersion, Value), String> {
    let bytes = read_bytes(path, MAX_CREDENTIAL_BYTES)?;
    let doc = serde_json::from_slice(&bytes).map_err(|_| "credential JSON is invalid")?;
    Ok((credential_version(&bytes), doc))
}

pub(crate) async fn refresh_file<C, V, R, F>(
    path: &Path,
    flavor: Flavor,
    reason: Reason,
    check: C,
    validate: V,
    remote: R,
) -> Result<FileResult, String>
where
    C: Fn() -> Result<(), String>,
    V: Fn(&Value) -> Result<(), String>,
    R: FnOnce(RefreshInput) -> F,
    F: Future<Output = Result<Value, RemoteError>>,
{
    check()?;
    let path = normalized_source(path)?;
    let key = path
        .to_str()
        .ok_or("credential path encoding is unsupported")?
        .to_owned();
    let mut state = lock_source(&key).await;
    check()?;
    let (version, doc) = read_doc(&path)?;
    validate(&doc)?;
    let flavor = flavor.resolve(&doc)?;
    let input = flavor.credentials(&doc)?;
    let identity = flavor.identity(&doc)?;
    state.check(version)?;
    let now = chrono::Utc::now().timestamp_millis();
    let rejected = match reason {
        Reason::Expiry => false,
        Reason::Rejected(v) => {
            if v != credential_version(input.access.as_bytes())
                && !input.access.is_empty()
                && input.expires > now
            {
                check()?;
                return Ok(FileResult { doc, outcome: None });
            }
            true
        }
    };
    let now = chrono::Utc::now().timestamp_millis();
    if !rejected
        && !input.access.is_empty()
        && (input.expires > now + flavor.buffer()
            || (state.completed == Some(version) && input.expires > now))
    {
        check()?;
        return Ok(FileResult { doc, outcome: None });
    }
    if input.refresh.is_empty() {
        return Err("token expired without a refresh token; sign in again".into());
    }
    check()?;
    let stage = match PrivateStage::new(&path) {
        Ok(s) => s,
        Err(_) if !rejected && !input.access.is_empty() && input.expires > now => {
            check()?;
            return Ok(FileResult { doc, outcome: None });
        }
        Err(_) => return Err("credentials cannot be staged safely; refresh was not sent".into()),
    };
    let mut attempt = begin_attempt(&key, flavor.family(), &input.refresh, version, &check).await?;
    // Recheck after the token-key queue as well as after the source queue.
    check()?;
    let (current, current_doc) = read_doc(&path)?;
    if current != version {
        return external_result(&path, &flavor, &identity, &check, &validate);
    }
    // A sidecar or other identity source can change independently of these
    // credential bytes while the token-level gate is awaited.
    validate(&current_doc)?;
    if flavor.identity(&current_doc)? != identity {
        return Err("credential source identity changed before refresh".into());
    }
    check()?;
    attempt.started();
    state.attempted(version);
    // Cancellation leaves the attempted latch in place, including an ambiguous
    // timeout after a request may already have reached the issuer.
    let response = remote(RefreshInput {
        refresh: input.refresh.clone(),
        doc: doc.clone(),
    })
    .await;
    let response = match response {
        Ok(v) => {
            state.save_failed(version);
            v
        }
        Err(err) => {
            if matches!(err, RemoteError::SucceededInvalid) {
                state.save_failed(version);
            }
            check()?;
            if read_doc(&path).map(|(v, _)| v != version).unwrap_or(false) {
                return external_result(&path, &flavor, &identity, &check, &validate);
            }
            return Err(match err {
                RemoteError::Rejected => {
                    "refresh was rejected; sign in again or update this source"
                }
                RemoteError::Unavailable => UNCERTAIN,
                RemoteError::SucceededInvalid => SAVE_FAILED,
            }
            .into());
        }
    };
    let rotated = response
        .get("refresh_token")
        .and_then(Value::as_str)
        .is_some_and(|v| !v.is_empty() && v != input.refresh);
    // Known success is recorded before parsing/identity/policy/commit. Even a
    // malformed success may already have consumed the old rotating token.
    let result = (|| {
        check()?;
        validate(&doc)?;
        let updated = flavor.patch(doc, &response)?;
        if flavor.identity(&updated)? != identity {
            return Err("returned account identity does not match the selected source".into());
        }
        validate(&updated)?;
        let bytes = serde_json::to_vec_pretty(&updated)
            .map_err(|_| "serialize refreshed credentials failed")?;
        let outcome = commit_stage(
            &path,
            ExpectedVersion::Present(version),
            stage,
            &bytes,
            &check,
        )?;
        if outcome == CommitOutcome::ChangedExternally {
            return external_result(&path, &flavor, &identity, &check, &validate);
        }
        state.completed(credential_version(&bytes));
        Ok(FileResult {
            doc: updated,
            outcome: Some(outcome),
        })
    })();
    if result.is_ok() {
        attempt.finish(rotated);
    }
    result.map_err(|e| format!("{SAVE_FAILED}: {e}"))
}
fn external_result<C, V>(
    path: &Path,
    flavor: &Flavor,
    identity: &Option<String>,
    check: C,
    validate: V,
) -> Result<FileResult, String>
where
    C: Fn() -> Result<(), String>,
    V: Fn(&Value) -> Result<(), String>,
{
    check()?;
    let (_, doc) = read_doc(path)?;
    validate(&doc)?;
    if flavor.identity(&doc)? != *identity {
        return Err(
            "credential source account changed externally; retry with the intended account".into(),
        );
    }
    let input = flavor.credentials(&doc)?;
    if input.access.is_empty() || input.expires <= chrono::Utc::now().timestamp_millis() {
        return Err("credential source changed externally; reread before retrying".into());
    }
    check()?;
    Ok(FileResult {
        doc,
        outcome: Some(CommitOutcome::ChangedExternally),
    })
}
struct Credentials {
    access: String,
    refresh: String,
    expires: i64,
}
fn string(doc: &Value, key: &str) -> String {
    doc.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
pub(crate) fn jwt_claims(token: &str) -> Option<Value> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}
pub(crate) fn jwt_subject(token: &str) -> Option<String> {
    jwt_claims(token)?
        .get("sub")?
        .as_str()
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}
pub(crate) fn codex_identity(doc: &Value) -> Result<Option<String>, String> {
    let tokens = doc
        .get("tokens")
        .filter(|v| v.is_object())
        .ok_or("auth.json has no OAuth tokens")?;
    let mut ids = Vec::new();
    if let Some(v) = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        ids.push(v.to_string());
    }
    for field in ["id_token", "access_token"] {
        if let Some(claims) = tokens
            .get(field)
            .and_then(Value::as_str)
            .and_then(jwt_claims)
        {
            if let Some(v) = claims
                .get("https://api.openai.com/auth")
                .and_then(|a| a.get("chatgpt_account_id"))
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
            {
                ids.push(v.to_string());
            }
        }
    }
    if ids.iter().any(|id| Some(id) != ids.first()) {
        return Err("conflicting Codex account identity".into());
    }
    Ok(ids.into_iter().next())
}
impl Flavor {
    fn family(&self) -> &'static str {
        match self {
            Self::Claude { .. } => "claude",
            Self::Codex => "codex",
            Self::Kimi => "kimi",
            Self::Grok { .. } => "grok",
        }
    }
    fn buffer(&self) -> i64 {
        if matches!(self, Self::Kimi) {
            300_000
        } else {
            60_000
        }
    }
    fn resolve(self, doc: &Value) -> Result<Self, String> {
        if let Self::Grok { entry } = &self {
            if entry.is_empty() {
                return Ok(Self::Grok {
                    entry: doc
                        .as_object()
                        .and_then(|v| v.keys().next())
                        .ok_or("Grok credential map is empty")?
                        .clone(),
                });
            }
        }
        Ok(self)
    }
    fn entry<'a>(&self, doc: &'a Value) -> Result<&'a Value, String> {
        let entry = match self {
            Self::Claude { .. } => doc.get("claudeAiOauth"),
            Self::Codex => doc.get("tokens"),
            Self::Kimi => Some(doc),
            Self::Grok { entry } => doc.get(entry),
        };
        entry
            .filter(|v| v.is_object())
            .ok_or_else(|| "required credential object missing".into())
    }
    fn identity(&self, doc: &Value) -> Result<Option<String>, String> {
        self.entry(doc)?;
        match self {
            Self::Codex => codex_identity(doc),
            Self::Claude { account } => Ok(account.clone()),
            Self::Grok { entry } => Ok(Some(entry.clone())),
            Self::Kimi => Ok(None),
        }
    }
    fn credentials(&self, doc: &Value) -> Result<Credentials, String> {
        let v = self.entry(doc)?;
        let (access, refresh, expires) = match self {
            Self::Claude { .. } => (
                string(v, "accessToken"),
                string(v, "refreshToken"),
                v.get("expiresAt").and_then(Value::as_i64).unwrap_or(0),
            ),
            Self::Codex => {
                let access = string(v, "access_token");
                let exp = jwt_claims(&access)
                    .and_then(|c| c.get("exp").and_then(Value::as_i64))
                    .unwrap_or(0)
                    .saturating_mul(1000);
                (access, string(v, "refresh_token"), exp)
            }
            Self::Kimi => {
                let mut exp = v
                    .get("expires_at")
                    .and_then(|x| {
                        x.as_f64()
                            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
                    })
                    .unwrap_or(0.0);
                if exp.abs() < 1e10 {
                    exp *= 1000.0;
                }
                (
                    string(v, "access_token"),
                    string(v, "refresh_token"),
                    exp as i64,
                )
            }
            Self::Grok { .. } => (
                string(v, "key"),
                string(v, "refresh_token"),
                v.get("expires_at")
                    .and_then(Value::as_str)
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|d| d.timestamp_millis())
                    .unwrap_or(i64::MAX),
            ),
        };
        Ok(Credentials {
            access,
            refresh,
            expires,
        })
    }
    pub(crate) fn patch(&self, mut doc: Value, response: &Value) -> Result<Value, String> {
        let access = response
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .ok_or("refresh response has no usable access token")?;
        if response
            .get("refresh_token")
            .is_some_and(|v| v.as_str().is_none_or(|s| s.trim().is_empty()))
        {
            return Err("refresh response has invalid refresh token".into());
        }
        if matches!(self, Self::Codex) {
            let claims = jwt_claims(access).ok_or("Codex returned a malformed access token")?;
            if claims.get("exp").and_then(Value::as_i64).unwrap_or(0)
                <= chrono::Utc::now().timestamp()
            {
                return Err("Codex returned an expired access token".into());
            }
            if let Some(id) = response.get("id_token") {
                if id.as_str().and_then(jwt_claims).is_none() {
                    return Err("Codex returned a malformed id token".into());
                }
            }
        }
        let expires = response
            .get("expires_in")
            .and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(3600);
        if !(1..=31_536_000).contains(&expires) {
            return Err("refresh response has invalid token expiry".into());
        }
        let expiry = chrono::Utc::now()
            .timestamp_millis()
            .saturating_add(expires.saturating_mul(1000));
        if let Self::Claude {
            account: Some(expected),
        } = self
        {
            if response
                .pointer("/account/uuid")
                .and_then(Value::as_str)
                .is_some_and(|v| v != expected)
            {
                return Err("returned Claude account identity mismatch".into());
            }
        }
        self.entry(&doc)?;
        let entry = match self {
            Self::Claude { .. } => doc.get_mut("claudeAiOauth").unwrap(),
            Self::Codex => doc.get_mut("tokens").unwrap(),
            Self::Kimi => &mut doc,
            Self::Grok { entry } => doc.get_mut(entry).unwrap(),
        };
        let (access_key, refresh_key) = match self {
            Self::Claude { .. } => ("accessToken", "refreshToken"),
            Self::Grok { .. } => ("key", "refresh_token"),
            _ => ("access_token", "refresh_token"),
        };
        entry[access_key] = Value::from(access);
        if let Some(refresh) = response.get("refresh_token").and_then(Value::as_str) {
            entry[refresh_key] = Value::from(refresh);
        }
        match self {
            Self::Claude { .. } => entry["expiresAt"] = Value::from(expiry),
            Self::Codex => {
                if let Some(id) = response.get("id_token") {
                    let id = id
                        .as_str()
                        .filter(|v| !v.is_empty())
                        .ok_or("refresh response has invalid id token")?;
                    entry["id_token"] = Value::from(id);
                }
                doc["last_refresh"] = Value::from(chrono::Utc::now().to_rfc3339());
            }
            Self::Kimi => {
                entry["expires_at"] = Value::from(expiry / 1000);
                entry["expires_in"] = Value::from(expires);
            }
            Self::Grok { .. } => {
                entry["expires_at"] = Value::from(
                    chrono::DateTime::from_timestamp_millis(expiry)
                        .ok_or("invalid expiry")?
                        .to_rfc3339(),
                )
            }
        }
        Ok(doc)
    }
}

/// The full identity is used for in-flight continuity; the short display ID is
/// only checked against the existing grant. Persistent full-identity pinning is
/// owned by account routing and is not supplied by an eight-character ID.
pub(crate) fn check_bound_directory(
    family: &str,
    directory: &Path,
    identity: Option<&str>,
) -> Result<(), String> {
    let permit = crate::access_policy::current_operation()
        .ok_or("credential operation has no account authorization")?;
    permit.check()?;
    if permit.account_id() == family {
        return Ok(());
    }
    let policy = permit.policy_snapshot()?;
    let binding = policy
        .account_bindings
        .get(permit.account_id())
        .ok_or("account binding missing")?;
    if binding.family != family
        || binding.directory.canonicalize().ok() != directory.canonicalize().ok()
    {
        return Err("credential source does not match selected account".into());
    }
    let full = identity.ok_or("selected account has no full identity")?;
    let short: String = full.chars().filter(|c| *c != '-').take(8).collect();
    if permit.account_id() != format!("{family}@{short}") {
        return Err("credential source account no longer matches selection".into());
    }
    Ok(())
}

/// Cursor's already-existing memory-only store. Cache entries are tied to the
/// logical SQLite pair and full subject, never to the whole changing database.
#[derive(Clone)]
struct MemoryEntry {
    source: CredentialVersion,
    subject: String,
    access: String,
    refresh: String,
    csv: Option<(i64, String)>,
}
fn memory_store() -> &'static Mutex<HashMap<String, MemoryEntry>> {
    static M: OnceLock<Mutex<HashMap<String, MemoryEntry>>> = OnceLock::new();
    M.get_or_init(Default::default)
}
pub(crate) struct EffectiveToken {
    pub access: String,
    pub source: CredentialVersion,
    pub subject: String,
}
fn memory_bind(key: &str, access: String, refresh: String) -> Result<MemoryEntry, String> {
    let source = pair_version(&access, &refresh);
    let subject = jwt_subject(&access).ok_or("Cursor token has no valid account subject")?;
    let mut cache = memory_store()
        .lock()
        .map_err(|_| "Cursor memory cache unavailable")?;
    let entry = cache.entry(key.into()).or_insert_with(|| MemoryEntry {
        source,
        subject: subject.clone(),
        access: access.clone(),
        refresh: refresh.clone(),
        csv: None,
    });
    if entry.source != source || entry.subject != subject {
        *entry = MemoryEntry {
            source,
            subject,
            access,
            refresh,
            csv: None,
        };
    }
    Ok(entry.clone())
}
fn effective(entry: MemoryEntry) -> EffectiveToken {
    EffectiveToken {
        access: entry.access,
        source: entry.source,
        subject: entry.subject,
    }
}
pub(crate) fn memory_csv(key: &str, source: CredentialVersion) -> Option<(i64, String)> {
    let cache = memory_store().lock().ok()?;
    let entry = cache.get(key)?;
    if entry.source != source {
        return None;
    }
    entry.csv.clone()
}
pub(crate) fn put_memory_csv(
    key: &str,
    source: CredentialVersion,
    at: i64,
    csv: String,
    check: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    check()?;
    let mut cache = memory_store()
        .lock()
        .map_err(|_| "Cursor memory cache unavailable")?;
    let entry = cache
        .get_mut(key)
        .filter(|v| v.source == source)
        .ok_or("Cursor source changed before CSV publication")?;
    check()?;
    entry.csv = Some((at, csv));
    Ok(())
}
pub(crate) async fn memory_access<C, S, R, F>(
    key: &str,
    read: S,
    rejected: Option<CredentialVersion>,
    check: C,
    remote: R,
) -> Result<EffectiveToken, String>
where
    C: Fn() -> Result<(), String>,
    S: Fn() -> Result<(String, String), String>,
    R: FnOnce(String) -> F,
    F: Future<Output = Result<Value, RemoteError>>,
{
    check()?;
    let mut state = lock_source(key).await;
    check()?;
    let (access, refresh) = read()?;
    check()?;
    let entry = memory_bind(key, access, refresh)?;
    state.check(entry.source)?;
    if rejected.is_none() || rejected != Some(credential_version(entry.access.as_bytes())) {
        check()?;
        return Ok(effective(entry));
    }
    if entry.refresh.is_empty() {
        return Err("Cursor has no refresh token; open Cursor and sign in".into());
    }
    let generation = pair_version(&entry.access, &entry.refresh);
    let mut attempt = begin_attempt(key, "cursor", &entry.refresh, generation, &check).await?;
    check()?;
    let (access, refresh) = read()?;
    if pair_version(&access, &refresh) != entry.source {
        return Err("Cursor source changed before refresh; reread it".into());
    }
    check()?;
    attempt.started();
    state.attempted(entry.source);
    let response = remote(entry.refresh.clone()).await;
    let response = match response {
        Ok(v) => {
            state.save_failed(entry.source);
            v
        }
        Err(err) => {
            if matches!(err, RemoteError::SucceededInvalid) {
                state.save_failed(entry.source);
            }
            check()?;
            let (access, refresh) = read()?;
            if pair_version(&access, &refresh) != entry.source {
                check()?;
                let current = memory_bind(key, access, refresh)?;
                if current.subject != entry.subject {
                    return Err("Cursor account changed externally".into());
                }
                return Ok(effective(current));
            }
            return Err(if matches!(err, RemoteError::SucceededInvalid) {
                SAVE_FAILED
            } else {
                UNCERTAIN
            }
            .into());
        }
    };
    let result: Result<EffectiveToken, String> = (|| {
        let token = response
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or("Cursor refresh returned no usable token")?;
        if jwt_subject(token).as_deref() != Some(entry.subject.as_str()) {
            return Err("Cursor returned account subject mismatch".into());
        }
        if let Some(expiry) = jwt_claims(token).and_then(|v| v.get("exp").cloned()) {
            if expiry
                .as_i64()
                .is_none_or(|v| v <= chrono::Utc::now().timestamp())
            {
                return Err("Cursor returned an expired or invalid access token".into());
            }
        }
        if response
            .get("expires_in")
            .is_some_and(|v| v.as_i64().is_none_or(|seconds| seconds <= 0))
        {
            return Err("Cursor returned invalid token lifetime".into());
        }
        let refresh = match response.get("refresh_token") {
            None => entry.refresh.clone(),
            Some(v) => v
                .as_str()
                .filter(|v| !v.is_empty())
                .ok_or("Cursor returned invalid refresh token")?
                .to_string(),
        };
        check()?;
        let (access, current_refresh) = read()?;
        if pair_version(&access, &current_refresh) != entry.source {
            check()?;
            let current = memory_bind(key, access, current_refresh)?;
            if current.subject != entry.subject {
                return Err("Cursor account changed externally".into());
            }
            // The validated response has a known rotation outcome even though
            // the external source wins publication. Do not leave an otherwise
            // reusable refresh token in the uncertain state.
            state.completed(current.source);
            attempt.finish(refresh != entry.refresh);
            return Ok(effective(current));
        }
        let new = MemoryEntry {
            source: entry.source,
            subject: entry.subject.clone(),
            access: token.into(),
            refresh: refresh.clone(),
            csv: entry.csv.clone(),
        };
        check()?;
        memory_store()
            .lock()
            .map_err(|_| "Cursor memory cache unavailable")?
            .insert(key.into(), new.clone());
        state.completed(entry.source);
        attempt.finish(refresh != entry.refresh);
        Ok(effective(new))
    })();
    result.map_err(|e| format!("{SAVE_FAILED}: {e}"))
}

pub(crate) struct CachedSource {
    pub version: CredentialVersion,
    pub access: String,
    pub refresh: Option<String>,
    pub expires: i64,
}
pub(crate) struct CachedToken {
    pub access: String,
    pub source: CredentialVersion,
}
/// Existing Antigravity access-only cache. The source fingerprint is persisted;
/// the keyring blob and refresh token never are. An old unbound cache is ignored.
pub(crate) async fn cached_access<C, S, R, F>(
    key: &str,
    path: &Path,
    read: S,
    rejected: Option<CredentialVersion>,
    check: C,
    remote: R,
) -> Result<CachedToken, String>
where
    C: Fn() -> Result<(), String>,
    S: Fn() -> Result<CachedSource, String>,
    R: FnOnce(String) -> F,
    F: Future<Output = Result<Value, RemoteError>>,
{
    check()?;
    let mut state = lock_source(key).await;
    check()?;
    let source = read()?;
    state.check(source.version)?;
    check()?;
    let parent = path.parent().ok_or("cache directory missing")?;
    std::fs::create_dir_all(parent).map_err(|_| "create Antigravity cache directory failed")?;
    check()?;
    let path =
        canonical_without_links(parent)?.join(path.file_name().ok_or("cache filename missing")?);
    let path = path.as_path();
    check()?;
    let expected = expected_version(path)?;
    let mut cache = match expected {
        ExpectedVersion::Absent => serde_json::json!({}),
        ExpectedVersion::Present(_) => {
            serde_json::from_slice::<Value>(&read_bytes(path, MAX_CREDENTIAL_BYTES)?)
                .ok()
                .filter(|v| v.is_object())
                .unwrap_or_else(|| serde_json::json!({}))
        }
    };
    let now = chrono::Utc::now().timestamp_millis();
    let cached = if cache.get("sourceFingerprint").and_then(Value::as_str)
        == Some(version_text(source.version).as_str())
        && cache
            .get("expiresAtMs")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            > now + 60_000
    {
        cache
            .get("accessToken")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    } else {
        None
    };
    let candidate = cached.or_else(|| {
        (!source.access.is_empty() && source.expires > now + 60_000).then(|| source.access.clone())
    });
    if let Some(access) = candidate {
        if rejected != Some(credential_version(access.as_bytes())) {
            check()?;
            return Ok(CachedToken {
                access,
                source: source.version,
            });
        }
    }
    let refresh = source
        .refresh
        .filter(|v| !v.is_empty())
        .ok_or("Antigravity token expired; open Antigravity to sign in")?;
    check()?;
    let parent = path.parent().ok_or("cache directory missing")?;
    std::fs::create_dir_all(parent).map_err(|_| "create Antigravity cache directory failed")?;
    check()?;
    let stage = PrivateStage::new(path)?;
    let mut attempt = begin_attempt(key, "antigravity", &refresh, source.version, &check).await?;
    check()?;
    if read()?.version != source.version {
        return Err("Antigravity source changed before refresh; reread it".into());
    }
    check()?;
    attempt.started();
    state.attempted(source.version);
    let response = remote(refresh.clone()).await;
    let response = match response {
        Ok(v) => {
            state.save_failed(source.version);
            v
        }
        Err(err) => {
            if matches!(err, RemoteError::SucceededInvalid) {
                state.save_failed(source.version);
            }
            check()?;
            let current = read()?;
            if current.version != source.version
                && !current.access.is_empty()
                && current.expires > now
            {
                check()?;
                return Ok(CachedToken {
                    access: current.access,
                    source: current.version,
                });
            }
            return Err(if matches!(err, RemoteError::SucceededInvalid) {
                SAVE_FAILED
            } else {
                UNCERTAIN
            }
            .into());
        }
    };
    let result: Result<CachedToken, String> = (|| {
        let access = response
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or("Antigravity refresh returned no usable token")?;
        if let Some(returned) = response.get("refresh_token") {
            if returned.as_str() != Some(refresh.as_str()) {
                return Err("Antigravity refresh credential rotated; open Antigravity to update its sign-in".into());
            }
        }
        let seconds = response
            .get("expires_in")
            .and_then(Value::as_i64)
            .unwrap_or(3600);
        if !(1..=31_536_000).contains(&seconds) {
            return Err("Antigravity refresh expiry is invalid".into());
        }
        check()?;
        if read()?.version != source.version {
            return Err("Antigravity source changed externally; cache not published".into());
        }
        cache["accessToken"] = Value::from(access);
        cache["expiresAtMs"] = Value::from(chrono::Utc::now().timestamp_millis() + seconds * 1000);
        cache["sourceFingerprint"] = Value::from(version_text(source.version));
        let bytes =
            serde_json::to_vec_pretty(&cache).map_err(|_| "serialize Antigravity cache failed")?;
        let outcome = commit_stage(path, expected, stage, &bytes, &check)?;
        if outcome == CommitOutcome::ChangedExternally {
            return Err("Antigravity cache changed externally; refresh not saved".into());
        }
        state.completed(source.version);
        attempt.finish(false);
        Ok(CachedToken {
            access: access.to_string(),
            source: source.version,
        })
    })();
    result.map_err(|e| format!("{SAVE_FAILED}: {e}"))
}
