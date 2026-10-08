//! Backend-owned consent. UI visibility filters can only narrow these grants.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::sync::watch;

pub const FAMILIES: &[&str] = &[
    "claude",
    "codex",
    "cursor",
    "opencode",
    "copilot",
    "grok",
    "devin",
    "minimax",
    "openrouter",
    "zai",
    "commandcode",
    "antigravity",
    "deepseek",
    "moonshot",
    "elevenlabs",
    "ollama",
    "codebuff",
    "kilo",
    "aihubmix",
    "qwen",
    "hermes",
    "kimi",
    "stepfun",
];
pub const MULTI_ACCOUNT_FAMILIES: &[&str] = &["claude", "codex", "opencode"];

/// Selection intent survives a missing directory and is separate from the
/// concrete canonical grants in scan_roots. Account consent never enables it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanSourceConfig {
    pub enabled: bool,
    pub mode: ScanSourceMode,
    #[serde(default)]
    pub custom_directories: Vec<PathBuf>,
    #[serde(default)]
    pub default_directories: Vec<PathBuf>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanSourceMode {
    #[default]
    Default,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountBinding {
    pub family: String,
    pub directory: PathBuf,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessPolicy {
    pub version: u32,
    pub enabled_families: BTreeSet<String>,
    pub enabled_accounts: BTreeSet<String>,
    #[serde(default)]
    pub scan_roots: BTreeMap<String, Vec<PathBuf>>,
    #[serde(default)]
    pub scan_sources: BTreeMap<String, ScanSourceConfig>,
    #[serde(default)]
    pub scan_epochs: BTreeMap<String, String>,
    #[serde(default)]
    pub regions: BTreeMap<String, String>,
    // A directory is remembered only after an explicit one-directory discovery.
    // Merely remembering it does not authorize later credential reads.
    #[serde(default)]
    pub account_bindings: BTreeMap<String, AccountBinding>,
}

impl Default for AccessPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            enabled_families: BTreeSet::new(),
            enabled_accounts: BTreeSet::new(),
            scan_roots: BTreeMap::new(),
            scan_sources: BTreeMap::new(),
            scan_epochs: BTreeMap::new(),
            regions: BTreeMap::new(),
            account_bindings: BTreeMap::new(),
        }
    }
}

impl AccessPolicy {
    pub fn from_config(config: &Value) -> Self {
        let mut policy = config
            .get("accessPolicy")
            .and_then(|p| serde_json::from_value::<Self>(p.clone()).ok())
            .filter(Self::is_valid)
            .unwrap_or_default();
        // Migrate only already-authorized roots, without resolving them or
        // discovering defaults. An explicit disabled intent always wins.
        for (source, directories) in &policy.scan_roots {
            if !directories.is_empty() {
                policy
                    .scan_sources
                    .entry(source.clone())
                    .or_insert_with(|| ScanSourceConfig {
                        enabled: true,
                        mode: ScanSourceMode::Custom,
                        custom_directories: directories.clone(),
                        default_directories: Vec::new(),
                    });
            }
        }
        policy
    }

    pub fn from_backend_config(config: &Value) -> Self {
        let mut policy = Self::from_config(config);
        if let Some(disabled) = config.get("disabled").and_then(Value::as_array) {
            policy
                .enabled_accounts
                .retain(|id| !disabled.iter().any(|d| d.as_str() == Some(id)));
        }
        policy
    }

    pub fn is_valid(&self) -> bool {
        self.version == 1
            && self
                .enabled_families
                .iter()
                .all(|f| FAMILIES.contains(&f.as_str()))
            && self
                .enabled_accounts
                .iter()
                .all(|id| self.known_account(id))
            && self.account_bindings.iter().all(|(id, b)| {
                MULTI_ACCOUNT_FAMILIES.contains(&b.family.as_str())
                    && id.starts_with(&format!("{}@", b.family))
                    && id.split_once('@').is_some_and(|(_, suffix)| {
                        suffix.len() == 8 && suffix.bytes().all(|c| c.is_ascii_alphanumeric())
                    })
                    && b.directory.is_absolute()
                    && !b.name.is_empty()
            })
    }

    pub fn known_account(&self, id: &str) -> bool {
        FAMILIES.contains(&id) || self.account_bindings.contains_key(id)
    }
    pub fn allows_discovery(&self, family: &str) -> bool {
        FAMILIES.contains(&family) && self.enabled_families.contains(family)
    }
    pub fn allows_account(&self, id: &str) -> bool {
        self.known_account(id)
            && self.allows_discovery(id.split('@').next().unwrap_or(id))
            && self.enabled_accounts.contains(id)
    }
    /// Legacy local-estimate cards must not be replayed as authenticated
    /// account quota after independent local consent has been withdrawn.
    pub fn allows_snapshot(&self, id: &str, plan: Option<&str>) -> bool {
        self.allows_account(id)
            && !(id.split('@').next() == Some("opencode")
                && plan.is_some_and(|p| p.to_ascii_lowercase().contains("this pc only")))
    }
    pub fn allows_metric(&self, id: &str, label: &str) -> bool {
        self.allows_account(id)
            && label != "Weekly capacity"
            && id.split('@').next() != Some("hermes")
            && !(id == "qwen" && matches!(label, "Requests today" | "Requests this month"))
            && (id != "kimi"
                || self.allows_account("moonshot")
                || !matches!(
                    label,
                    "API" | "Credits used" | "Balance" | "Vouchers" | "Cash"
                ))
    }
    pub fn read_account<T>(&self, id: &str, read: impl FnOnce() -> T) -> Option<T> {
        self.allows_account(id).then(read)
    }
    pub fn set_family(&mut self, family: &str, enabled: bool) -> Result<(), String> {
        if !FAMILIES.contains(&family) {
            return Err("Unknown provider family".into());
        }
        if enabled {
            self.enabled_families.insert(family.into());
        } else {
            self.enabled_families.remove(family);
            self.enabled_accounts
                .retain(|id| id.split('@').next() != Some(family));
        }
        Ok(())
    }
    pub fn set_account(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        if !self.known_account(id) {
            return Err("Discover this account directory first".into());
        }
        if enabled {
            if !self.allows_discovery(id.split('@').next().unwrap_or(id)) {
                return Err("Enable the provider family first".into());
            }
            self.enabled_accounts.insert(id.into());
        } else {
            self.enabled_accounts.remove(id);
        }
        Ok(())
    }
}

struct State {
    policy: AccessPolicy,
    revision: u64,
    suspended: usize,
}
pub struct AccessRuntime {
    state: RwLock<State>,
    changes: watch::Sender<u64>,
    checker: Option<Box<dyn Fn() -> AccessPolicy + Send + Sync>>,
}
impl AccessRuntime {
    pub fn new(policy: AccessPolicy) -> Self {
        Self {
            state: RwLock::new(State {
                policy,
                revision: 0,
                suspended: 0,
            }),
            changes: watch::channel(0).0,
            checker: None,
        }
    }
    /// The desktop bridge injects its backend config loader. Tests inject a
    /// synthetic source, so policy checks never need a real user directory.
    pub fn with_checker(checker: impl Fn() -> AccessPolicy + Send + Sync + 'static) -> Self {
        let mut runtime = Self::new(checker());
        runtime.checker = Some(Box::new(checker));
        runtime
    }
    pub fn snapshot(&self) -> (AccessPolicy, u64) {
        if let Some(checker) = &self.checker {
            self.update(checker());
        }
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        (state.policy.clone(), state.revision)
    }
    pub fn update(&self, policy: AccessPolicy) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        if state.policy != policy {
            state.policy = policy;
            state.revision = state.revision.wrapping_add(1);
            self.changes.send_replace(state.revision);
        }
    }
    /// Final synchronous publication/write boundary. Refresh the injected
    /// source first, then exclude concurrent observed revision changes while
    /// applying the effect. The closure must not re-enter this runtime or await.
    pub fn commit_if_current<T>(
        &self,
        revision: u64,
        effect: impl FnOnce(&AccessPolicy) -> T,
    ) -> Option<T> {
        self.snapshot();
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        (state.suspended == 0 && state.revision == revision).then(|| effect(&state.policy))
    }
    /// Fence a synchronous mode/cache transaction. No new account operation or
    /// publication is admitted until every nested suspension has ended. The
    /// guard must be dropped before awaiting a native UI callback.
    pub fn suspend(&self) -> AccessSuspension<'_> {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.suspended += 1;
        state.revision = state.revision.wrapping_add(1);
        self.changes.send_replace(state.revision);
        AccessSuspension(self)
    }
    pub fn invalidate(&self) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.revision = state.revision.wrapping_add(1);
        self.changes.send_replace(state.revision);
    }
    /// Hot-loop guard after a full backend observation. App-owned changes
    /// update this revision immediately; explicit source reloads remain at
    /// operation start/final-publication checkpoints.
    pub fn is_current_cached(&self, revision: u64) -> bool {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        state.suspended == 0 && state.revision == revision
    }
    pub fn is_current(&self, revision: u64) -> bool {
        self.snapshot();
        self.is_current_cached(revision)
    }
    pub fn permit(self: &Arc<Self>, id: &str, disabled: &[String]) -> Option<AccessPermit> {
        let (policy, revision) = self.snapshot();
        let mut narrowed: BTreeSet<String> = disabled.iter().cloned().collect();
        let mut execution_check = None;
        if let Ok(parent) = CURRENT_OPERATION.try_with(Clone::clone) {
            if !Arc::ptr_eq(self, &parent.runtime)
                || parent.revision != revision
                || parent.check().is_err()
            {
                return None;
            }
            narrowed.extend(parent.disabled);
            execution_check = parent.execution_check;
        }
        (self.is_current_cached(revision) && policy.allows_account(id) && !narrowed.contains(id)
            && execution_check.as_ref().is_none_or(|check: &OperationCheck| check(id)))
            .then(|| AccessPermit {
                runtime: self.clone(),
                account: id.into(),
                revision,
                disabled: narrowed,
                execution_check,
            })
    }
    pub async fn run_account_at<T>(
        self: &Arc<Self>,
        id: &str,
        disabled: &[String],
        revision: u64,
        future: impl Future<Output = T>,
    ) -> Option<T> {
        let permit = self.permit(id, disabled)?;
        if permit.revision != revision {
            return None;
        }
        permit.run(future).await
    }
    /// Optional scheduling gate for automatic quota work. This does not change
    /// consent or cancel a running request. Existing I/O checkpoints (and nested
    /// account permits) consult it before starting additional work.
    pub async fn run_account_at_checked<T>(
        self: &Arc<Self>, id: &str, disabled: &[String], revision: u64,
        execution_check: Option<OperationCheck>, future: impl Future<Output = T>,
    ) -> Option<T> {
        let mut permit = self.permit(id, disabled)?;
        if permit.revision != revision { return None; }
        if execution_check.is_some() { permit.execution_check = execution_check; }
        permit.run(future).await
    }
    pub async fn run_account<T>(
        self: &Arc<Self>,
        id: &str,
        disabled: &[String],
        future: impl Future<Output = T>,
    ) -> Option<T> {
        self.permit(id, disabled)?.run(future).await
    }
}

pub struct AccessSuspension<'a>(&'a AccessRuntime);
impl Drop for AccessSuspension<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state.write().unwrap_or_else(|e| e.into_inner());
        state.suspended -= 1;
        // Reject work that captured the suspended revision, even on failure.
        state.revision = state.revision.wrapping_add(1);
        self.0.changes.send_replace(state.revision);
    }
}

pub type OperationCheck = Arc<dyn Fn(&str) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct AccessPermit {
    runtime: Arc<AccessRuntime>,
    account: String,
    revision: u64,
    disabled: BTreeSet<String>,
    execution_check: Option<OperationCheck>,
}
tokio::task_local! { static CURRENT_OPERATION: AccessPermit; }
impl AccessPermit {
    pub fn account_id(&self) -> &str {
        &self.account
    }

    /// Policy and revision come from the same checked runtime snapshot.
    pub fn policy_snapshot(&self) -> Result<AccessPolicy, String> {
        self.check()?;
        let (policy, revision) = self.runtime.snapshot();
        if revision == self.revision
            && self.runtime.is_current_cached(revision)
            && policy.allows_account(&self.account)
        {
            Ok(policy)
        } else {
            Err("Account access was revoked; retry after explicit consent".into())
        }
    }

    pub fn check(&self) -> Result<(), String> {
        self.check_authorization()?;
        if self.execution_check.as_ref().is_some_and(|check| !check(&self.account)) {
            return Err("Automatic quota refresh is off; refresh this account manually".into());
        }
        Ok(())
    }
    fn check_authorization(&self) -> Result<(), String> {
        let (policy, revision) = self.runtime.snapshot();
        if revision == self.revision
            && self.runtime.is_current_cached(revision)
            && policy.allows_account(&self.account)
        {
            Ok(())
        } else {
            Err("Account access was revoked; retry after explicit consent".into())
        }
    }
    pub async fn run<T>(&self, future: impl Future<Output = T>) -> Option<T> {
        let mut changes = self.runtime.changes.subscribe();
        self.check().ok()?;
        CURRENT_OPERATION
            .scope(self.clone(), async {
                tokio::select! {
                    biased;
                    _ = changes.changed() => None,
                    value = future => self.check_authorization().ok().map(|_| value),
                }
            })
            .await
    }
}

/// Credential-bearing request/writeback sites must call this immediately before
/// acting. Missing task context is denied. Public unauthenticated price downloads
/// deliberately do not use this hook. Child tasks must explicitly propagate it.
pub fn check_current_operation() -> Result<(), String> {
    CURRENT_OPERATION
        .try_with(AccessPermit::check)
        .unwrap_or_else(|_| Err("Credential operation has no account authorization".into()))
}
pub fn current_operation() -> Option<AccessPermit> {
    CURRENT_OPERATION.try_with(Clone::clone).ok()
}

/// Extra-account discovery reads only files inside the explicitly selected
/// directory. Reject symlink/reparse escapes rather than silently broadening it.
pub fn contained_file(directory: &Path, filename: &str) -> Result<PathBuf, String> {
    let root = directory
        .canonicalize()
        .map_err(|_| "Account directory is unavailable")?;
    let file = directory
        .join(filename)
        .canonicalize()
        .map_err(|_| "Account file is unavailable")?;
    if file.parent() != Some(root.as_path()) {
        return Err("Account file escapes the selected directory".into());
    }
    Ok(file)
}

pub fn retired_id(id: &str) -> bool {
    matches!(id.split('@').next(), Some("onenewapi" | "sub2api"))
}

pub fn sanitize_retired_config(cfg: &mut Value) {
    for key in ["disabled", "trayProviders"] {
        if let Some(ids) = cfg.get_mut(key).and_then(Value::as_array_mut) {
            ids.retain(|id| id.as_str().is_some_and(|id| !retired_id(id)));
        }
    }
    if cfg
        .pointer("/pinned/provider")
        .and_then(Value::as_str)
        .is_some_and(retired_id)
    {
        cfg["pinned"] = Value::Null;
    }
    if let Some(ids) = cfg
        .pointer_mut("/layout/providerOrder")
        .and_then(Value::as_array_mut)
    {
        ids.retain(|id| id.as_str().is_some_and(|id| !retired_id(id)));
    }
    if let Some(providers) = cfg
        .pointer_mut("/layout/providers")
        .and_then(Value::as_object_mut)
    {
        providers.retain(|id, _| !retired_id(id));
    }
    if let Some(obj) = cfg.as_object_mut() {
        obj.retain(|key, _| {
            !key.to_lowercase().contains("onenewapi") && !key.to_lowercase().contains("sub2api")
        });
    }
}

pub fn private_config_dir(base: &Path) -> PathBuf {
    base.join("PanePrivate")
}

impl AccessPermit {
    /// Resolve the account selected for a reset-credit operation.
    pub fn read_bound_target<T>(
        &self,
        family: &str,
        reader: impl FnOnce(&str, &AccountBinding) -> Option<T>,
    ) -> Option<T> {
        let policy = self.policy_snapshot().ok()?;
        if self.account.split('@').next() != Some(family) || self.disabled.contains(&self.account) {
            return None;
        }
        let binding = policy.account_bindings.get(&self.account)?;
        if binding.family != family {
            return None;
        }
        self.check().ok()?;
        // No runtime lock spans credential IO: its reader may perform another
        // permit checkpoint (including the forthcoming refresh/writeback hooks).
        let value = reader(&self.account, binding)?;
        self.check().ok()?;
        Some(value)
    }
}
