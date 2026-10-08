//! Account-bound network configuration. Selection never grants credential access.
use crate::access_policy::{AccessPolicy, AccessRuntime, FAMILIES};
use crate::network_policy::{region_choices, validate_selection, RegionChoice};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeCatalogEntry {
    pub id: String,
    pub family: String,
    pub choices: &'static [RegionChoice],
    pub selected: Option<String>,
    pub default_selection: Option<&'static str>,
    pub allow_local_origin: bool,
}

fn account_family<'a>(policy: &'a AccessPolicy, id: &'a str) -> Result<&'a str, String> {
    if !policy.is_valid() {
        return Err("Malformed account binding or access policy".into());
    }
    if FAMILIES.contains(&id) {
        return Ok(id);
    }
    policy
        .account_bindings
        .get(id)
        .map(|binding| binding.family.as_str())
        .ok_or_else(|| "Unknown account; identify its directory first".into())
}

pub fn catalog(policy: &AccessPolicy) -> Vec<ModeCatalogEntry> {
    FAMILIES
        .iter()
        .copied()
        .chain(policy.account_bindings.keys().map(String::as_str))
        .filter_map(|id| {
            let family = account_family(policy, id).ok()?;
            let choices = region_choices(family);
            if choices.is_empty() {
                return None;
            }
            Some(ModeCatalogEntry {
                id: id.into(),
                family: family.into(),
                choices,
                selected: policy.regions.get(id).cloned(),
                default_selection: (family == "antigravity").then_some("cloud"),
                allow_local_origin: family == "ollama",
            })
        })
        .collect()
}

pub struct ModeChange {
    config: Value,
    family: String,
    snapshot_ids: Vec<String>,
    changed: bool,
}

/// Validate against the backend's known account binding and exact mode catalog.
/// This reads config only, including when both family and account are disabled.
pub fn prepare(config: &Value, id: &str, selection: &str) -> Result<ModeChange, String> {
    let mut policy = AccessPolicy::from_config(config);
    let family = account_family(&policy, id)?.to_string();
    validate_selection(&family, selection)?;
    let changed = policy.regions.get(id).map(String::as_str) != Some(selection);
    policy.regions.insert(id.into(), selection.into());
    let mut config = config.clone();
    config["accessPolicy"] = serde_json::json!(policy);
    let snapshot_ids = if id == "moonshot" {
        vec![id.into(), "kimi".into()]
    } else {
        vec![id.into()]
    };
    Ok(ModeChange {
        config,
        family,
        snapshot_ids,
        changed,
    })
}
impl ModeChange {
    pub fn config(&self) -> &Value {
        &self.config
    }
    pub fn family(&self) -> &str {
        &self.family
    }
    pub fn snapshot_ids(&self) -> &[String] {
        &self.snapshot_ids
    }
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// Caller holds the existing publication/config transaction locks. Clear
    /// derived files before saving the new selection: on any failure the old
    /// mode remains selected. Cleared caches can safely be rebuilt. No await.
    pub fn commit(
        self,
        runtime: &AccessRuntime,
        clear: impl FnOnce() -> Result<(), String>,
        persist: impl FnOnce(&Value) -> Result<(), String>,
        success: impl FnOnce(),
    ) -> Result<Value, String> {
        if !self.changed {
            return Ok(self.config);
        }
        let _suspension = runtime.suspend();
        clear()?;
        persist(&self.config)?;
        runtime.update(AccessPolicy::from_backend_config(&self.config));
        success();
        Ok(self.config)
    }
}

/// Remove only derived identity stamps, never opening an authentication file.
/// Unreadable/malformed cache files block changing the mode until cleanup works.
pub fn forget_identities_in(dir: &Path, ids: &[String]) -> Result<(), String> {
    let path = dir.join("cache_identities.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("read cached identities: {error}")),
    };
    let mut doc: Value = serde_json::from_str(&raw).map_err(|_| "Malformed cached identities")?;
    let map = doc.as_object_mut().ok_or("Malformed cached identities")?;
    for id in ids {
        map.remove(id);
    }
    crate::private_file::atomic_write(&path, &doc.to_string())
        .map_err(|error| format!("clear cached identities: {error}"))
}
