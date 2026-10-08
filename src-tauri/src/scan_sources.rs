//! Explicit source selection and metadata-only availability, independent of accounts.
//! Default candidates are finite known log locations, never account discovery.
//! A grant freezes canonical paths; later environment or symlink changes cannot
//! silently replace them. Missing defaults may appear at that exact frozen path.
use crate::access_policy::{AccessPolicy, ScanSourceConfig, ScanSourceMode};
use crate::scan_policy::{self, ScanPolicy};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Component, Path, PathBuf},
};

#[derive(Default)]
pub struct SourceEnvironment {
    pub home: Option<PathBuf>,
    pub vars: BTreeMap<String, PathBuf>,
}
impl SourceEnvironment {
    /// Read directory environment variables only, never files or credentials.
    pub fn current() -> Self {
        let vars = [
            "CLAUDE_CONFIG_DIR",
            "CODEX_HOME",
            "XDG_DATA_HOME",
            "APPDATA",
            "LOCALAPPDATA",
            "PI_CODING_AGENT_DIR",
            "PI_CODING_AGENT_SESSION_DIR",
            "STEP_CODING_AGENT_DIR",
            "GROK_HOME",
            "KIMI_SHARE_DIR",
            "KIMI_CODE_HOME",
        ]
        .into_iter()
        .filter_map(|key| {
            std::env::var_os(key)
                .filter(|v| !v.is_empty())
                .map(|value| (key.to_string(), PathBuf::from(value)))
        })
        .collect();
        Self {
            home: dirs::home_dir(),
            vars,
        }
    }
    fn variable(&self, name: &str) -> Result<Option<PathBuf>, String> {
        self.vars
            .get(name)
            .map(|path| {
                if path.is_absolute()
                    && !path.components().any(|c| matches!(c, Component::ParentDir))
                {
                    Ok(path.clone())
                } else {
                    Err(format!(
                        "{name} must name an absolute directory without parent traversal"
                    ))
                }
            })
            .transpose()
    }
    fn home(&self, suffix: &str) -> Result<PathBuf, String> {
        self.home
            .as_ref()
            .filter(|path| path.is_absolute())
            .map(|p| p.join(suffix))
            .ok_or_else(|| "Home directory is unavailable; choose a custom log directory".into())
    }
    fn base(&self, var: &str, suffix: &str) -> Result<PathBuf, String> {
        match self.variable(var)? {
            Some(path) => Ok(path),
            None => self.home(suffix),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSourceStatus {
    pub source: String,
    pub enabled: bool,
    pub mode: ScanSourceMode,
    pub custom_directories: Vec<PathBuf>,
    pub directories: Vec<DirectoryStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct DirectoryStatus {
    pub path: PathBuf,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Finite paths matching the existing adapters. SQLite sources use their data
/// directory because their adapter opens only named databases and safe sidecars.
/// Claude/Codex/Pi/Kimi/Qwen grants stop at their leaf log trees, excluding auth.
pub fn default_directories(source: &str, env: &SourceEnvironment) -> Result<Vec<PathBuf>, String> {
    let paths = match source {
        "claude" => vec![env.base("CLAUDE_CONFIG_DIR", ".claude")?.join("projects")],
        "codex" => {
            let p = env.base("CODEX_HOME", ".codex")?;
            vec![p.join("sessions"), p.join("archived_sessions")]
        }
        "opencode" => vec![env.base("XDG_DATA_HOME", ".local/share")?.join("opencode")],
        "pi" => {
            let pi = match env.variable("PI_CODING_AGENT_SESSION_DIR")? {
                Some(path) => path,
                None => env
                    .base("PI_CODING_AGENT_DIR", ".pi/agent")?
                    .join("sessions"),
            };
            vec![pi, env.home(".omp/agent/sessions")?]
        }
        "stepcode" => vec![env
            .base("STEP_CODING_AGENT_DIR", ".stepcode/agent")?
            .join("sessions")],
        "grok" => vec![env.base("GROK_HOME", ".grok")?.join("logs")],
        "devin" => {
            let p = match env.variable("APPDATA")? {
                Some(path) => path,
                None => env.base("XDG_DATA_HOME", ".local/share")?,
            };
            vec![p.join("devin/cli")]
        }
        // sqlite.db and v2/sqlite/runtime-state.sqlite are both fixed names
        // under this root; the adapter never enumerates auth or config files.
        "minimax" => vec![env.home(".minimax")?],
        "hermes" => vec![env
            .variable("LOCALAPPDATA")?
            .ok_or("Hermes data location is unavailable; choose the directory containing state.db")?
            .join("hermes")],
        // Both supported CLI generations write wire.jsonl under sessions.
        // https://moonshotai.github.io/kimi-cli/en/configuration/data-locations.html
        // https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/guides/sessions.md
        "kimi" => vec![
            env.base("KIMI_CODE_HOME", ".kimi-code")?.join("sessions"),
            env.base("KIMI_SHARE_DIR", ".kimi")?.join("sessions"),
        ],
        "qwen" => vec![env.home(".qwen/usage")?],
        _ => return Err("Unknown log source".into()),
    };
    let mut unique = Vec::new();
    for path in paths {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    Ok(unique)
}

/// Resolve the existing ancestor without reading or enumerating its contents.
/// A not-yet-created leaf can be granted, but its eventual canonical spelling
/// must remain identical. A later symlink to any other target fails closed.
fn freeze_default(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path.parent().is_none()
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err("Choose an absolute log directory, not a filesystem root".into());
    }
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                if !resolved.is_dir() {
                    return Err("Log location is not a directory".into());
                }
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                if resolved.parent().is_none() {
                    return Err("Filesystem roots cannot be log locations".into());
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Do not turn a dangling link into a future directory grant.
                if fs::symlink_metadata(ancestor).is_ok() {
                    return Err("Log directory is an unavailable link".into());
                }
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or("Log directory is unavailable")?
                        .to_os_string(),
                );
                ancestor = ancestor.parent().ok_or("Log directory is unavailable")?;
            }
            Err(_) => return Err("Log directory is unavailable".into()),
        }
    }
}
fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
fn check_other_sources(
    policy: &AccessPolicy,
    source: &str,
    paths: &[PathBuf],
) -> Result<(), String> {
    // No filesystem reads of unrelated grants: offline sources cannot block
    // another source, but their frozen path ownership is still reserved.
    if policy.scan_roots.iter().any(|(other, roots)| {
        other != source
            && paths
                .iter()
                .any(|path| roots.iter().any(|root| overlap(path, root)))
    }) {
        Err("Different log sources must use separate directories".into())
    } else {
        Ok(())
    }
}
fn epoch() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|_| "Cannot create a new source-grant generation")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Both IPC routes must distinguish remembered canonical grants from new
/// user input. Remembered locations survive OFF and mode changes, so neither
/// route may re-resolve their exact spelling into a replacement target.
fn validate_custom_directories(
    source: &str,
    previous: &ScanSourceConfig,
    requested: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    if requested.is_empty() || requested.len() > 32 {
        return Err("Choose between one and 32 custom log directories".into());
    }
    let mut custom = Vec::new();
    for path in requested {
        let saved = previous
            .custom_directories
            .iter()
            .chain(&previous.default_directories)
            .any(|p| p.as_os_str() == path.as_os_str());
        let roots = BTreeMap::from([(source.to_string(), vec![path.clone()])]);
        let validated = if saved {
            ScanPolicy::from_saved(roots)
        } else {
            ScanPolicy::new(roots)
        }?;
        for p in validated.roots.into_values().flatten() {
            if !custom.contains(&p) {
                custom.push(p);
            }
        }
    }
    Ok(custom)
}

/// Prepare all validation before mutating. A failed custom save leaves the
/// complete prior selection and authority untouched, with no default fallback.
pub fn configure(
    policy: &mut AccessPolicy,
    env: &SourceEnvironment,
    source: &str,
    enabled: bool,
    mode: &str,
    directories: Option<Vec<PathBuf>>,
) -> Result<(), String> {
    if !scan_policy::SOURCES.contains(&source) {
        return Err("Unknown log source".into());
    }
    let mode = match mode {
        "default" => ScanSourceMode::Default,
        "custom" => ScanSourceMode::Custom,
        _ => return Err("Unknown log location mode".into()),
    };
    if mode == ScanSourceMode::Default && directories.is_some() {
        return Err("Default mode does not accept custom directories".into());
    }
    let previous = policy.scan_sources.get(source).cloned().unwrap_or_default();
    let mut next = previous.clone();
    next.enabled = enabled;
    next.mode = mode;
    if let Some(requested) = directories.as_ref() {
        let custom = validate_custom_directories(source, &previous, requested)?;
        check_other_sources(policy, source, &custom)?;
        next.custom_directories = custom;
    }
    // Repeated ON or availability refresh must not resolve new default targets.
    let authority_unchanged = if enabled {
        policy
            .scan_roots
            .get(source)
            .is_some_and(|roots| roots == selected(&next))
    } else {
        !policy.scan_roots.contains_key(source) && !policy.scan_epochs.contains_key(source)
    };
    if previous == next && directories.is_none() && authority_unchanged {
        return Ok(());
    }
    let roots = if !enabled {
        Vec::new()
    } else {
        match mode {
            ScanSourceMode::Default => {
                if next.default_directories.is_empty() {
                    next.default_directories = default_directories(source, env)?
                        .iter()
                        .map(|p| freeze_default(p))
                        .collect::<Result<_, _>>()?;
                }
                next.default_directories.clone()
            }
            ScanSourceMode::Custom => {
                if next.custom_directories.is_empty() {
                    return Err("Save a custom log directory first".into());
                }
                // Validate the exact remembered target on re-enable; never retarget.
                ScanPolicy::from_saved(BTreeMap::from([(
                    source.to_string(),
                    next.custom_directories.clone(),
                )]))?;
                next.custom_directories.clone()
            }
        }
    };
    check_other_sources(policy, source, &roots)?;
    let old_roots = policy.scan_roots.get(source).cloned().unwrap_or_default();
    let changed = next != previous || roots != old_roots;
    let generation = if enabled && changed {
        Some(epoch()?)
    } else {
        None
    };
    policy.scan_sources.insert(source.to_string(), next);
    if enabled {
        policy.scan_roots.insert(source.to_string(), roots);
        if let Some(generation) = generation {
            policy.scan_epochs.insert(source.to_string(), generation);
        }
    } else {
        policy.scan_roots.remove(source);
        policy.scan_epochs.remove(source);
    }
    Ok(())
}

/// Compatibility for previous clients: replace the custom selection and keep
/// durable intent coherent, including the old removal-only offline behavior.
pub fn set_legacy(
    policy: &mut AccessPolicy,
    source: &str,
    directories: Vec<PathBuf>,
) -> Result<(), String> {
    let active = policy.scan_roots.get(source).cloned().unwrap_or_default();
    let removal_only = directories.iter().all(|path| {
        active
            .iter()
            .any(|saved| saved.as_os_str() == path.as_os_str())
    });
    let directories = if removal_only {
        // Removing authority must work even when a retained path is offline or
        // retargeted. The old helper preserves exact spellings in this branch.
        directories
    } else {
        let previous = policy.scan_sources.get(source).cloned().unwrap_or_default();
        validate_custom_directories(source, &previous, &directories)?
    };
    scan_policy::update_source_grant(
        &mut policy.scan_roots,
        &mut policy.scan_epochs,
        source,
        directories,
    )?;
    let selected = policy.scan_roots.get(source).cloned().unwrap_or_default();
    let config = policy.scan_sources.entry(source.to_string()).or_default();
    config.enabled = !selected.is_empty();
    config.mode = ScanSourceMode::Custom;
    if config.enabled {
        config.custom_directories = selected;
    }
    Ok(())
}

fn selected(config: &ScanSourceConfig) -> &[PathBuf] {
    match config.mode {
        ScanSourceMode::Default => &config.default_directories,
        ScanSourceMode::Custom => &config.custom_directories,
    }
}
/// Keep good roots even when another root/source is missing or retargeted.
/// This filters existing authority only; it never synthesizes or canonicalizes
/// grant-time input into a replacement target.
pub fn runtime_policy(policy: &AccessPolicy) -> ScanPolicy {
    let mut roots = BTreeMap::new();
    for (source, paths) in &policy.scan_roots {
        let Some(config) = policy.scan_sources.get(source).filter(|c| c.enabled) else {
            continue;
        };
        if paths.len() > 32 || !scan_policy::SOURCES.contains(&source.as_str()) {
            continue;
        }
        let mut valid = Vec::new();
        for path in paths {
            if !selected(config)
                .iter()
                .any(|p| p.as_os_str() == path.as_os_str())
                || check_other_sources(policy, source, std::slice::from_ref(path)).is_err()
            {
                continue;
            }
            if ScanPolicy::from_saved(BTreeMap::from([(source.clone(), vec![path.clone()])]))
                .is_ok()
            {
                valid.push(path.clone());
            }
        }
        if !valid.is_empty() {
            roots.insert(source.clone(), valid);
        }
    }
    // Each retained root has been validated independently. I/O checkpoints
    // revalidate again, without one late filesystem change blanking other roots.
    let mut result = ScanPolicy::default();
    result.roots = roots;
    result
}
fn directory_status(path: PathBuf, frozen: bool) -> DirectoryStatus {
    if !path.is_absolute() || path.parent().is_none() {
        return DirectoryStatus {
            path,
            status: "unavailable".into(),
            error: Some("Choose an absolute log directory, not a filesystem root".into()),
        };
    }
    let outcome = match path.canonicalize() {
        Ok(resolved) if frozen && resolved.as_os_str() != path.as_os_str() => Err(
            "Saved log directory resolves elsewhere; choose its new location explicitly"
                .to_string(),
        ),
        Ok(resolved) if !resolved.is_dir() || resolved.parent().is_none() => {
            Err("Log location is not a directory".into())
        }
        Ok(_) => Ok("found"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if frozen
                && !freeze_default(&path)
                    .is_ok_and(|expected| expected.as_os_str() == path.as_os_str())
            {
                Err("Saved log directory is an unavailable or retargeted location".into())
            } else {
                Ok("notFound")
            }
        }
        Err(_) => Err("Log directory is unavailable".into()),
    };
    match outcome {
        Ok(status) => DirectoryStatus {
            path,
            status: status.into(),
            error: None,
        },
        Err(error) => DirectoryStatus {
            path,
            status: "unavailable".into(),
            error: Some(error),
        },
    }
}
pub fn catalog(policy: &AccessPolicy, env: &SourceEnvironment) -> Vec<ScanSourceStatus> {
    scan_policy::SOURCES
        .iter()
        .map(|&source| {
            let config = policy.scan_sources.get(source).cloned().unwrap_or_default();
            let preview =
                config.mode == ScanSourceMode::Default && config.default_directories.is_empty();
            let (paths, mut error) = if preview {
                match default_directories(source, env) {
                    Ok(paths) => (paths, None),
                    Err(error) => (Vec::new(), Some(error)),
                }
            } else {
                (selected(&config).to_vec(), None)
            };
            let directories: Vec<_> = paths
                .into_iter()
                .map(|p| {
                    if config.enabled {
                        let granted = policy.scan_roots.get(source).is_some_and(|roots| {
                            roots.iter().any(|root| root.as_os_str() == p.as_os_str())
                        });
                        let problem = if !granted {
                            Some("Log location has no saved authorization".into())
                        } else {
                            check_other_sources(policy, source, std::slice::from_ref(&p)).err()
                        };
                        if let Some(error) = problem {
                            return DirectoryStatus {
                                path: p,
                                status: "unavailable".into(),
                                error: Some(error),
                            };
                        }
                    }
                    directory_status(p, !preview)
                })
                .collect();
            if error.is_none() {
                error = directories.iter().find_map(|d| d.error.clone());
            }
            ScanSourceStatus {
                source: source.to_string(),
                enabled: config.enabled,
                mode: config.mode,
                custom_directories: config.custom_directories,
                directories,
                error,
            }
        })
        .collect()
}
