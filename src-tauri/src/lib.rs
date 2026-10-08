mod cache_fingerprint;
mod access_policy;
mod snapshot;
mod usage_publication;
mod network_policy;
mod provider_modes;
mod provider_auto_refresh;
mod network_transport;
mod alerts;
mod capacity;
mod httpapi;
mod i18n;
mod pricing;
mod providers;
mod spend;
mod scan_policy;
mod scan_sources;
mod private_file;
mod credential_refresh;
mod tray_projection;
mod widget;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};

/// Last-good snapshots older than this are too misleading to show or to
/// use as a stand-in for a live Moonshot card.
const SNAPSHOT_CACHE_MS: i64 = 24 * 60 * 60 * 1000;
/// One failed cycle isn't "Outdated": vendors hiccup routinely.
const STALE_GRACE_MS: i64 = 3 * 60 * 1000;

// ---------------------------------------------------------------------------
// App settings, stored at %APPDATA%\Pane\config.json
// ---------------------------------------------------------------------------

fn config_path_in(dir: &Path) -> PathBuf {
    dir.join("config.json")
}

/// A parse failure here once silently reset all settings to defaults, so
/// failures are now logged durably and the last good copy is used instead.
fn note_config_error(context: &str) {
    eprintln!("[pane] {context}");
    // Tests run against temp dirs; they must never append into the
    // developer's real config-error.log.
    if cfg!(test) {
        return;
    }
    let line = format!(
        "{} {}\r\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        context
    );
    let path = providers::config_dir().join("config-error.log");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

fn parse_config_file(path: &PathBuf) -> Result<Value, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    // Tolerate a UTF-8 BOM (Notepad and PowerShell 5.1 both write one).
    let value: Value =
        serde_json::from_str(raw.trim_start_matches('\u{feff}')).map_err(|e| format!("parse: {e}"))?;
    // `[]` / `null` / `"x"` parse, but Pane's contract is an object.
    // Treating those as unreadable lets load fall back to the backup
    // and stops a save from copying junk over the last good copy.
    if !value.is_object() {
        return Err("not an object".into());
    }
    Ok(value)
}

fn load_config() -> Value {
    load_config_from(&providers::config_dir())
}

fn load_config_from(dir: &Path) -> Value {
    let path = config_path_in(dir);
    if !path.exists() {
        return json!({});
    }
    match parse_config_file(&path) {
        Ok(cfg) => cfg,
        Err(e) => {
            note_config_error(&format!("config.json unreadable ({e}) — trying backup"));
            let backup = dir.join("config.json.bak");
            match parse_config_file(&backup) {
                Ok(mut cfg) => {
                    // Recovery of display settings must never restore an old consent grant.
                    cfg["accessPolicy"] = json!(access_policy::AccessPolicy::default());
                    cfg
                },
                Err(e2) => {
                    note_config_error(&format!("config.json.bak also failed ({e2}) — defaults"));
                    json!({})
                }
            }
        }
    }
}

fn config_with_defaults(mut cfg: Value) -> Value {
    if !cfg.is_object() {
        cfg = json!({});
    }
    let obj = cfg.as_object_mut().unwrap();
    // Out-of-the-box experience: 1-min refresh, pacing always visible,
    // all three quota alerts on, dark + compact. (Autostart defaults on
    // in setup; tray icon defaults to Auto via pinned = null.)
    obj.entry("refreshMinutes").or_insert(json!(1));
    obj.entry("disabled").or_insert(json!([]));
    obj.entry("pinned").or_insert(Value::Null);
    obj.entry("trayProviders").or_insert(json!([]));
    obj.entry("pacingAlways").or_insert(json!(true));
    obj.entry("notifyAlmostOut").or_insert(json!(true));
    obj.entry("notifyCuttingClose").or_insert(json!(true));
    obj.entry("notifyWillRunOut").or_insert(json!(true));
    obj.entry("notifyReset").or_insert(json!(true));
    obj.entry("spendTab").or_insert(json!("today"));
    obj.entry("spendMetric").or_insert(json!("cost"));
    obj.entry("showUsed").or_insert(json!(false));
    obj.entry("resetExact").or_insert(json!(false));
    obj.entry("timeFormat").or_insert(json!("auto"));
    obj.entry("layout").or_insert(Value::Null);
    obj.entry("appearance").or_insert(json!("dark"));
    obj.entry("density").or_insert(json!("compact"));
    obj.entry("minimal").or_insert(json!(false));
    obj.entry("glassEffects").or_insert(json!(true));
    obj.entry("shortcut").or_insert(json!(""));
    obj.entry("proxy")
        .or_insert(json!({ "enabled": false, "url": "" }));
    obj.entry("showTotalSpend").or_insert(json!(true));
    obj.entry("welcomeDismissed").or_insert(json!(false));
    // Empty = "never recorded": the frontend uses it to tell a fresh
    // install (no What's-new popup) from an update (popup with the notes).
    obj.entry("lastSeenVersion").or_insert(json!(""));
    // Star-prompt bookkeeping: firstSeenMs is stamped by the frontend on
    // the first config load that finds it 0, so it doubles as the install
    // age; on upgrades it stamps at the first launch of the version that
    // introduced it, so the first ask lands a few days after updating —
    // intended, since a just-installed build should earn the ask. The
    // rest rate-limit and retire the prompt.
    obj.entry("firstSeenMs").or_insert(json!(0));
    obj.entry("starPromptDone").or_insert(json!(false));
    obj.entry("starPromptDay").or_insert(json!(""));
    obj.entry("starPromptDayCount").or_insert(json!(0));
    obj.entry("starPromptLastMs").or_insert(json!(0));
    obj.entry("reduceAnimations").or_insert(json!(false));
    obj.entry("locale").or_insert(json!("auto"));
    // StepFun's plan tier pick — null = "Not set" (Credits row stays an
    // estimate-only text row instead of a bar against a monthly pool).
    obj.entry("stepfunPlanCredits").or_insert(Value::Null);
    obj.entry("widgetMode").or_insert(json!(false));
    obj.entry("widgetCollapsed").or_insert(json!(false));
    obj.entry("widgetLocked").or_insert(json!(false));
    // Extra Codex session folders the user synced in from other machines.
    obj.entry("codexExtraDirs").or_insert(json!([]));
    cfg["providerAutoRefresh"] = provider_auto_refresh::normalized(&cfg);
    cfg["accessPolicy"] = json!(access_policy::AccessPolicy::from_config(&cfg));
    sanitize_retired_config(&mut cfg);
    cfg
}

use access_policy::{retired_id, sanitize_retired_config};

pub(crate) fn access_runtime() -> &'static Arc<access_policy::AccessRuntime> {
    static RUNTIME: OnceLock<Arc<access_policy::AccessRuntime>> = OnceLock::new();
    RUNTIME.get_or_init(|| Arc::new(access_policy::AccessRuntime::with_checker(||
        access_policy::AccessPolicy::from_backend_config(&load_config()),
    )))
}

/// Capture backend-only source consent and fence every later local read and
/// result with that revision. A missing/unavailable root fails closed.
pub(crate) fn local_scan_policy() -> scan_policy::ScanPolicy {
    let runtime = access_runtime().clone();
    let (policy, revision) = runtime.snapshot();
    scan_sources::runtime_policy(&policy).with_revision(revision)
        .with_epochs(policy.scan_epochs).with_check(move || runtime.is_current_cached(revision))
}

#[derive(serde::Serialize)]
struct SpendResult { revision: u64, rows: Vec<spend::ProviderSpend>, #[serde(rename = "preserveCursor")] preserve_cursor: bool }

pub(crate) fn usage_publications() -> &'static usage_publication::UsagePublication {
    static STORE: OnceLock<usage_publication::UsagePublication> = OnceLock::new();
    STORE.get_or_init(Default::default)
}

fn empty_usage_result() -> usage_publication::UsageResult {
    usage_publication::UsageResult::empty(access_runtime().snapshot().1)
}

fn retain_authorized_snapshots(snapshots: &mut Vec<providers::Snapshot>, policy: &access_policy::AccessPolicy) {
    usage_publication::filter_snapshots(snapshots, policy);
}

// Recheck on the native UI thread, where the actual icon update occurs.
// Never hold the publication mutex while awaiting UI dispatch: a native
// callback waiting on that same mutex could otherwise deadlock the window.
async fn publish_authorized_main_tray(
    app: tauri::AppHandle,
    revision: Option<u64>,
    projection: Option<tray_projection::TrayProjectionConfig>,
    strip_active: bool,
) -> Result<(), String> {
    if let Some(revision) = revision {
        usage_publications().apply(access_runtime(), revision, |_, _| ())
            .ok_or_else(|| "Stale tray publication".to_string())?;
    }
    let handle = app.clone();
    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    app.run_on_main_thread(move || {
        let result = if let (Some(revision), Some(projection)) = (revision, projection) {
            usage_publications().apply(access_runtime(), revision, |snapshots, _| {
                let main = tray_projection::project_main_tray(snapshots, &projection, strip_active);
                apply_main_tray_projection(&handle, &main)
            }).unwrap_or_else(|| Err("Stale native main-tray publication".into()))
        } else {
            apply_main_tray_projection(&handle, &tray_projection::MainTrayProjection {
                icon_mode: tray_projection::MainTrayIconMode::Logo,
                remaining_percentages: Vec::new(), tooltip: "Pane Private".into(),
            })
        };
        let _ = sender.try_send(result);
    }).map_err(|error| format!("dispatch main tray: {error}"))?;
    receiver.recv().await.ok_or_else(|| "Main tray update did not complete".to_string())?
}

async fn clear_consent_trays(app: tauri::AppHandle) -> Result<(), String> {
    usage_publication::clear_both(
        update_tray_strip(app.clone(), Vec::new(), None),
        publish_authorized_main_tray(app, None, None, false),
    ).await
}

fn change_access_policy(change: impl FnOnce(&mut access_policy::AccessPolicy) -> Result<(), String>) -> Result<Value, String> {
    let _publication = KEY_CARD_PUBLICATION.lock().unwrap_or_else(|e| e.into_inner());
    let _config = CONFIG_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut cfg = config_with_defaults(load_config());
    let mut policy = access_policy::AccessPolicy::from_config(&cfg);
    change(&mut policy)?;
    if !policy.is_valid() { return Err("Malformed account binding or access policy".into()); }
    cfg["accessPolicy"] = json!(&policy);
    persist_config_in(&providers::config_dir(), &cfg)?;
    access_runtime().snapshot();
    // Revocation immediately clears already-published values, in addition to
    // cancelling async work and rejecting its late publication.
    usage_publications().clear();
    spend::invalidate_published();
    let mut cache = last_ok().lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, cached| policy.allows_snapshot(&cached.snap.id, cached.snap.plan.as_deref()));
    for c in cache.values_mut() { usage_publication::filter_snapshot(&mut c.snap, &policy); }
    let _ = persist_last_ok(&cache);
    fail_state().lock().unwrap_or_else(|e| e.into_inner()).retain(|id, _| policy.allows_account(id));
    Ok(cfg)
}

#[tauri::command]
async fn set_provider_family(app: tauri::AppHandle, family: String, enabled: bool) -> Result<Value, String> {
    let mut cfg = change_access_policy(|policy| policy.set_family(&family, enabled))?;
    if let Err(error) = clear_consent_trays(app).await { cfg["accessWarning"] = json!(error); }
    Ok(cfg)
}

#[tauri::command]
async fn set_provider_account(app: tauri::AppHandle, account: String, enabled: bool) -> Result<Value, String> {
    let mut cfg = change_access_policy(|policy| policy.set_account(&account, enabled))?;
    if let Err(error) = clear_consent_trays(app).await { cfg["accessWarning"] = json!(error); }
    Ok(cfg)
}

#[tauri::command]
fn get_provider_modes() -> Vec<provider_modes::ModeCatalogEntry> {
    provider_modes::catalog(&access_policy::AccessPolicy::from_config(&load_config()))
}

fn set_provider_region_inner(id: &str, region: &str) -> Result<Value, String> {
    let mut mutation = KeyCardMutationGuard::begin(Vec::new());
    let _config = CONFIG_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = config_with_defaults(load_config());
    let change = provider_modes::prepare(&cfg, id, region)?;
    let family = change.family().to_string();
    let snapshot_ids = change.snapshot_ids().to_vec();
    if change.changed() { mutation.track(snapshot_ids.clone()); }
    let dir = providers::config_dir();
    change.commit(access_runtime(), || {
        usage_publications().clear();
        spend::invalidate_published();
        // Run every cleanup even if one fails. Saving the changed mode is
        // conditional on all derived state being gone, including on disk.
        let results = [
            forget_provider_snapshots_inner(&snapshot_ids, true),
            provider_modes::forget_identities_in(&dir, &snapshot_ids),
            providers::forget_credit_baselines_in(&dir, &[id.to_string()]),
            if family == "minimax" { providers::minimax::forget_remembered_tier_in(&dir) } else { Ok(()) },
        ];
        let errors: Vec<String> = results.into_iter().filter_map(Result::err).collect();
        if errors.is_empty() { Ok(()) } else {
            Err(format!("Mode was not changed; clearing previous account data failed: {}", errors.join("; ")))
        }
    }, |next| persist_config_in(&dir, next), || {
        if family == "qwen" { providers::qwen::reset_quota_cooldown(); }
    })
}

#[tauri::command]
async fn set_provider_region(app: tauri::AppHandle, id: String, region: String) -> Result<Value, String> {
    // The synchronous transaction releases every mutex before native dispatch.
    let mut cfg = set_provider_region_inner(&id, &region)?;
    if let Err(error) = clear_consent_trays(app).await { cfg["accessWarning"] = json!(error); }
    Ok(cfg)
}

#[tauri::command]
fn get_scan_sources() -> Vec<scan_sources::ScanSourceStatus> {
    scan_sources::catalog(&access_policy::AccessPolicy::from_config(&load_config()),
        &scan_sources::SourceEnvironment::current())
}

#[tauri::command]
async fn configure_scan_source(app: tauri::AppHandle, source: String, enabled: bool,
    mode: String, directories: Option<Vec<String>>) -> Result<Value, String> {
    let environment = scan_sources::SourceEnvironment::current();
    let mut cfg = change_access_policy(|policy| scan_sources::configure(policy, &environment,
        &source, enabled, &mode, directories.map(|paths| paths.into_iter().map(PathBuf::from).collect())))?;
    if let Err(error) = clear_consent_trays(app).await { cfg["accessWarning"] = json!(error); }
    Ok(cfg)
}

#[tauri::command]
async fn set_scan_source(app: tauri::AppHandle, source: String, directories: Vec<String>) -> Result<Value, String> {
    let mut cfg = change_access_policy(|policy| {
        scan_sources::set_legacy(policy, &source, directories.iter().map(PathBuf::from).collect())
    })?;
    if let Err(error) = clear_consent_trays(app).await { cfg["accessWarning"] = json!(error); }
    Ok(cfg)
}

#[tauri::command]
async fn reset_provider_access(app: tauri::AppHandle) -> Result<Value, String> {
    let mut cfg = change_access_policy(|policy| { *policy = access_policy::AccessPolicy::default(); Ok(()) })?;
    providers::qwen::reset_quota_cooldown();
    if let Err(error) = clear_consent_trays(app).await { cfg["accessWarning"] = json!(error); }
    Ok(cfg)
}

#[tauri::command]
fn discover_provider_account(family: String, directory: String) -> Result<Value, String> {
    // This explicit action authorizes identification of exactly one directory.
    // It never enumerates the user's home or enables the resulting account.
    change_access_policy(|policy| {
        if !policy.allows_discovery(&family) { return Err("Enable this provider family first".into()); }
        let dir = PathBuf::from(directory).canonicalize().map_err(|_| "Account directory is unavailable")?;
        let (id, name) = match family.as_str() {
            "claude" => providers::claude::account_at_directory(&dir).map(|a| (a.id, a.name)),
            "codex" => providers::codex::account_at_directory(&dir).map(|a| (a.id, a.name)),
            "opencode" => providers::opencode::account_at_directory(&dir).map(|a| (a.id, a.name)),
            _ => return Err("This provider uses its default account".into()),
        }.ok_or("No supported account found inside that directory")?;
        if policy.account_bindings.get(&id).is_some_and(|b| b.directory != dir) {
            return Err("This account ID is already bound to another directory".into());
        }
        policy.account_bindings.insert(id, access_policy::AccountBinding { family, directory: dir, name });
        Ok(())
    })
}

#[tauri::command]
fn system_ui_locale() -> &'static str {
    i18n::system_ui_locale()
}

#[tauri::command]
fn get_config() -> Value {
    config_with_defaults(load_config())
}

/// Every key config.json may hold — the same set config_with_defaults seeds.
/// set_config drops anything else so a compromised frontend can't stash
/// arbitrary data in the config file.
const CONFIG_KEYS: &[&str] = &[
    // Not seeded by config_with_defaults (the autostart plugin is the
    // source of truth at runtime) but persisted here so setup() can apply
    // the user's choice on launch.
    "autostart",
    "refreshMinutes",
    "providerAutoRefresh",
    "disabled",
    "pinned",
    "trayProviders",
    "pacingAlways",
    "notifyAlmostOut",
    "notifyCuttingClose",
    "notifyWillRunOut",
    "notifyReset",
    "spendMetric",
    "spendTab",
    "showUsed",
    "resetExact",
    "timeFormat",
    "layout",
    "appearance",
    "density",
    "minimal",
    "glassEffects",
    "shortcut",
    "proxy",
    "showTotalSpend",
    "welcomeDismissed",
    "lastSeenVersion",
    "firstSeenMs",
    "starPromptDone",
    "starPromptDay",
    "starPromptDayCount",
    "starPromptLastMs",
    "reduceAnimations",
    "locale",
    "stepfunPlanCredits",
    "widgetMode",
    "widgetCollapsed",
    "widgetLocked",
    "codexExtraDirs",
];

static CONFIG_WRITE: Mutex<()> = Mutex::new(());
static CONFIG_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn apply_config_patch(cfg: &mut Value, patch: &Value) {
    if let (Some(target), Some(source)) = (cfg.as_object_mut(), patch.as_object()) {
        for (k, v) in source {
            if CONFIG_KEYS.contains(&k.as_str()) {
                if k == "providerAutoRefresh" {
                    // A single-family update must not reset unrelated saved
                    // preferences. Malformed whole-map writes still fail closed.
                    let mut next = target.get(k).cloned()
                        .unwrap_or_else(|| provider_auto_refresh::normalized(&json!({})));
                    if let (Some(values), Some(changes)) = (next.as_object_mut(), v.as_object()) {
                        for (family, value) in changes { values.insert(family.clone(), value.clone()); }
                    } else { next = v.clone(); }
                    target.insert(k.clone(), provider_auto_refresh::normalized(&json!({"providerAutoRefresh": next})));
                } else if k == "locale" {
                    let ok = matches!(v.as_str(), Some("auto" | "en" | "zh" | "ru"));
                    target.insert(k.clone(), if ok { v.clone() } else { json!("auto") });
                } else {
                    target.insert(k.clone(), v.clone());
                }
            } else {
                eprintln!("[pane] set_config: ignoring unknown key '{k}'");
            }
        }
    }
}

/// Injectable filesystem operations for config persistence, so failure
/// paths (disk full, locked file, failed replace) are repeatable tests
/// instead of best-effort permission games.
#[derive(Clone, Copy)]
struct ConfigPersistIo {
    write_tmp: fn(&Path, &str) -> std::io::Result<()>,
    replace: fn(&Path, &Path) -> std::io::Result<()>,
}

impl ConfigPersistIo {
    fn real() -> Self {
        Self {
            write_tmp: |path, raw| std::fs::write(path, raw),
            // std::fs::rename replaces an existing destination on Windows
            // (MoveFileEx REPLACE_EXISTING), so the swap is atomic-ish.
            replace: |tmp, path| std::fs::rename(tmp, path),
        }
    }
}

/// Commit order for one config save (callers hold CONFIG_WRITE):
///   1. write the new config to a unique temp file — failure cleans the
///      temp and leaves the main file and backup untouched;
///   2. refresh the backup from the main file, but ONLY while the main
///      file still parses — a corrupt main must never clobber the last
///      good backup, because that backup is the only thing a corrupt
///      main recovers from;
///   3. replace the main file with the temp — failure removes the temp
///      and both old files survive.
///
/// After every step at least one parseable config exists: the old main,
/// the backup, or (once step 1 succeeded) the temp itself.
fn persist_config_at(dir: &Path, cfg: &Value, io: ConfigPersistIo) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create config dir: {e}"))?;
    let path = config_path_in(dir);
    let backup = dir.join("config.json.bak");
    let tmp = dir.join(format!(
        "config.{}.{}.tmp",
        std::process::id(),
        CONFIG_TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let raw = serde_json::to_string_pretty(cfg).unwrap_or_default();
    if let Err(e) = (io.write_tmp)(&tmp, &raw) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("write config: {e}"));
    }
    if path.exists() && parse_config_file(&path).is_ok() {
        // Copy into a temp, then rename over the backup. A failed
        // mid-copy must not truncate the last good .bak in place.
        let bak_tmp = dir.join(format!(
            "config.bak.{}.{}.tmp",
            std::process::id(),
            CONFIG_TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let refresh = (|| {
            std::fs::copy(&path, &bak_tmp)?;
            std::fs::rename(&bak_tmp, &backup)
        })();
        if let Err(e) = refresh {
            let _ = std::fs::remove_file(&bak_tmp);
            // Not fatal: the new config is already in `tmp`, and the
            // previous backup (if any) is still intact.
            note_config_error(&format!("config backup refresh failed ({e})"));
        }
    }
    if let Err(e) = (io.replace)(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("replace config: {e}"));
    }
    Ok(())
}

fn persist_config_in(dir: &Path, cfg: &Value) -> Result<(), String> {
    persist_config_at(dir, cfg, ConfigPersistIo::real())
}

fn set_config_in(dir: &Path, patch: Value) -> Result<Value, String> {
    let _guard = CONFIG_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut cfg = config_with_defaults(load_config_from(dir));
    apply_config_patch(&mut cfg, &patch);
    sanitize_retired_config(&mut cfg);
    persist_config_in(dir, &cfg)?;
    Ok(cfg)
}

fn set_config_inner(patch: Value) -> Result<Value, String> {
    let old_disabled = config_with_defaults(load_config()).get("disabled").cloned();
    let cfg = set_config_in(&providers::config_dir(), patch)?;
    if old_disabled != cfg.get("disabled").cloned() { access_runtime().invalidate(); }
    let disabled = cfg.get("disabled").and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_default();
    httpapi::forget_disabled_snapshots(&disabled);
    Ok(cfg)
}

#[tauri::command]
fn set_config(app: tauri::AppHandle, patch: Value) -> Result<Value, String> {
    let _publication = KEY_CARD_PUBLICATION.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = set_config_inner(patch)?;
    apply_tray_locale(&app, &cfg);
    Ok(cfg)
}

fn apply_tray_locale(app: &tauri::AppHandle, cfg: &Value) {
    let next = i18n::resolved_locale(cfg);
    static LAST: Mutex<Option<&'static str>> = Mutex::new(None);
    let Ok(mut last) = LAST.lock() else {
        return;
    };
    if *last == Some(next) {
        return;
    }
    *last = Some(next);
    drop(last);
    let Ok(quit) = MenuItem::with_id(app, "quit", i18n::quit_label(cfg), true, None::<&str>) else {
        return;
    };
    let Ok(menu) = Menu::with_items(app, &[&quit]) else {
        return;
    };
    if let Some(tray) = app.tray_by_id("tray") {
        let _ = tray.set_menu(Some(menu));
    }
}

/// Settings validation for "Codex session folders": the path must be
/// absolute and point at a directory that exists.
#[tauri::command]
fn check_dir(path: String) -> Result<(), String> {
    let dir = Path::new(path.trim());
    if !dir.is_absolute() {
        return Err("not an absolute path".into());
    }
    if !dir.is_dir() {
        return Err("folder does not exist".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Start with Windows
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    // Remember the choice so startup knows whether to re-assert it.
    let _ = set_config_inner(json!({ "autostart": enabled }));
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Tray icon with the pinned metric drawn onto it
// ---------------------------------------------------------------------------

// 4x6 pixel digit font, one nibble per row (bit 3 = leftmost pixel).
const DIGIT_FONT: [[u8; 6]; 10] = [
    [0x6, 0x9, 0x9, 0x9, 0x9, 0x6], // 0
    [0x2, 0x6, 0x2, 0x2, 0x2, 0x7], // 1
    [0x6, 0x9, 0x1, 0x2, 0x4, 0xF], // 2
    [0xE, 0x1, 0x6, 0x1, 0x9, 0x6], // 3
    [0x2, 0x6, 0xA, 0xF, 0x2, 0x2], // 4
    [0xF, 0x8, 0xE, 0x1, 0x9, 0x6], // 5
    [0x6, 0x8, 0xE, 0x9, 0x9, 0x6], // 6
    [0xF, 0x1, 0x2, 0x2, 0x4, 0x4], // 7
    [0x6, 0x9, 0x6, 0x9, 0x9, 0x6], // 8
    [0x6, 0x9, 0x9, 0x7, 0x1, 0x6], // 9
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrayInk {
    foreground: [u8; 4],
    outline: [u8; 4],
}

const TRAY_INK_LIGHT: TrayInk = TrayInk {
    foreground: [20, 24, 33, 255],
    outline: [255, 255, 255, 220],
};
const TRAY_INK_DARK: TrayInk = TrayInk {
    foreground: [255, 255, 255, 255],
    outline: [0, 0, 0, 200],
};
const TRAY_INK_FALLBACK: TrayInk = TrayInk {
    foreground: [255, 255, 255, 255],
    outline: [0, 0, 0, 230],
};

fn tray_ink_for_theme(light: Option<u32>) -> TrayInk {
    match light {
        Some(1) => TRAY_INK_LIGHT,
        Some(0) => TRAY_INK_DARK,
        _ => TRAY_INK_FALLBACK,
    }
}

#[cfg(windows)]
fn system_uses_light_theme() -> Option<u32> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
        REG_DWORD, REG_VALUE_TYPE,
    };

    // Read once per redraw; a successful open always has one matching close.
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows_core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            None,
            KEY_READ,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut kind = REG_VALUE_TYPE::default();
        let mut bytes = [0u8; 4];
        let mut len = bytes.len() as u32;
        let result = RegQueryValueExW(
            key,
            windows_core::w!("SystemUsesLightTheme"),
            None,
            Some(&mut kind),
            Some(bytes.as_mut_ptr()),
            Some(&mut len),
        );
        let _ = RegCloseKey(key);
        (result == ERROR_SUCCESS && kind == REG_DWORD && len == 4)
            .then(|| u32::from_le_bytes(bytes))
    }
}

#[cfg(not(windows))]
fn system_uses_light_theme() -> Option<u32> {
    None
}

/// Renders one or two numbers (0-100) on a 32x32 RGBA tray icon.
fn draw_tray_numbers_with(values: &[u32], ink: TrayInk) -> Vec<u8> {
    const SIZE: usize = 32;
    let scale = 2usize;
    let glyph_w = 4 * scale;
    let gap = scale;

    let mut mask = [false; SIZE * SIZE];
    let rows: &[usize] = if values.len() >= 2 { &[1, 18] } else { &[10] };

    for (value, y0) in values.iter().zip(rows) {
        let digits: Vec<usize> = value
            .to_string()
            .chars()
            .filter_map(|c| c.to_digit(10).map(|d| d as usize))
            .collect();
        let text_w = digits.len() * glyph_w + digits.len().saturating_sub(1) * gap;
        let x0 = (SIZE.saturating_sub(text_w)) / 2;

        for (i, d) in digits.iter().enumerate() {
            let gx = x0 + i * (glyph_w + gap);
            for (row, bits) in DIGIT_FONT[*d].iter().enumerate() {
                for col in 0..4 {
                    if bits & (0x8 >> col) != 0 {
                        for sy in 0..scale {
                            for sx in 0..scale {
                                let x = gx + col * scale + sx;
                                let y = y0 + row * scale + sy;
                                if x < SIZE && y < SIZE {
                                    mask[y * SIZE + x] = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    // Outline anywhere adjacent to a text pixel.
    for y in 0..SIZE {
        for x in 0..SIZE {
            if mask[y * SIZE + x] {
                continue;
            }
            let near = (-1i32..=1).any(|dy| {
                (-1i32..=1).any(|dx| {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    nx >= 0
                        && ny >= 0
                        && (nx as usize) < SIZE
                        && (ny as usize) < SIZE
                        && mask[ny as usize * SIZE + nx as usize]
                })
            });
            if near {
                let p = (y * SIZE + x) * 4;
                rgba[p..p + 4].copy_from_slice(&ink.outline);
            }
        }
    }
    for y in 0..SIZE {
        for x in 0..SIZE {
            if mask[y * SIZE + x] {
                let p = (y * SIZE + x) * 4;
                rgba[p..p + 4].copy_from_slice(&ink.foreground);
            }
        }
    }
    rgba
}

fn draw_tray_numbers(values: &[u32]) -> Vec<u8> {
    draw_tray_numbers_with(values, tray_ink_for_theme(system_uses_light_theme()))
}

fn apply_main_tray_projection(
    app: &tauri::AppHandle,
    projection: &tray_projection::MainTrayProjection,
) -> Result<(), String> {
    let tray = app
        .tray_by_id("tray")
        .ok_or_else(|| "main tray icon is unavailable".to_string())?;
    tray.set_tooltip(Some(&projection.tooltip))
        .map_err(|error| format!("set main tray tooltip: {error}"))?;
    match projection.icon_mode {
        tray_projection::MainTrayIconMode::Logo => {
            let default = app
                .default_window_icon()
                .ok_or_else(|| "default Pane icon is unavailable".to_string())?;
            tray.set_icon(Some(default.clone()))
                .map_err(|error| format!("set main tray logo: {error}"))?;
        }
        tray_projection::MainTrayIconMode::Numbers => {
            let icon = tauri::image::Image::new_owned(
                draw_tray_numbers(&projection.remaining_percentages),
                32,
                32,
            );
            tray.set_icon(Some(icon))
                .map_err(|error| format!("set main tray numbers: {error}"))?;
        }
    }
    if let Ok(mut slot) = last_main_tray().lock() {
        slot.lefts = projection.remaining_percentages.clone();
        slot.tooltip = projection.tooltip.clone();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Mac-style tray strip: a [provider logo][live numbers] icon pair per
// selected provider. The UI rasterizes each SVG logo to 32x32 RGBA (the
// webview already has the icons) and sends the pixels here.
// ---------------------------------------------------------------------------

struct LastMainTray {
    lefts: Vec<u32>,
    tooltip: String,
}

fn last_main_tray() -> &'static Mutex<LastMainTray> {
    static S: OnceLock<Mutex<LastMainTray>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(LastMainTray {
            lefts: Vec::new(),
            tooltip: String::from("Pane"),
        })
    })
}

fn last_strip() -> &'static Mutex<Vec<StripEntry>> {
    static S: OnceLock<Mutex<Vec<StripEntry>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Vec::new()))
}

fn tray_strip_apply_lock() -> &'static tauri::async_runtime::Mutex<()> {
    static LOCK: OnceLock<tauri::async_runtime::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tauri::async_runtime::Mutex::new(()))
}

#[derive(Clone, serde::Deserialize)]
struct StripEntry {
    id: String,
    logo: Vec<u8>, // 32x32 RGBA
    labels: Vec<String>,
    #[serde(skip)]
    values: Vec<u32>,
    #[serde(skip)]
    tooltip: String,
}

/// Every provider family that may appear in the tray strip. Frontend
/// strip ids are validated against this before becoming tray icon ids,
/// including `family@account` cards. Stale family-level strip icons are
/// removed for exactly this set.
const STRIP_PROVIDER_IDS: [&str; 23] = [
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

fn derive_strip_entries(entries: &[StripEntry], snapshots: &[providers::Snapshot], policy: &access_policy::AccessPolicy) -> Vec<StripEntry> {
    entries.iter().filter_map(|entry| {
        let view = usage_publication::strip_view(snapshots, policy, &entry.id, &entry.labels)?;
        let mut entry = entry.clone();
        entry.values = view.values;
        entry.tooltip = view.tooltip;
        Some(entry)
    }).collect()
}

async fn update_tray_strip(app: tauri::AppHandle, mut entries: Vec<StripEntry>, revision: Option<u64>) -> Result<(), String> {
    validate_strip_entries(&entries)?;
    let _guard = tray_strip_apply_lock().lock().await;
    if let Some(revision) = revision {
        entries = usage_publications().apply(access_runtime(), revision, |snapshots, policy| derive_strip_entries(&entries, snapshots, policy))
            .ok_or_else(|| "Stale tray-strip publication".to_string())?;
    }
    let previous = last_strip()
        .lock()
        .map(|slot| slot.clone())
        .unwrap_or_default();
    let reset_ids = strip_reset_ids(&previous, &entries);
    let rebuild_order = !reset_ids.is_empty();
    let result = apply_tray_strip(
        app.clone(),
        entries.clone(),
        reset_ids,
        rebuild_order,
        revision,
    )
    .await;
    if result.is_err() {
        if clear_tray_strip_icons(app, &previous, &entries)
            .await
            .is_ok()
        {
            if let Ok(mut slot) = last_strip().lock() {
                slot.clear();
            }
        }
        return result;
    }
    let Ok(mut slot) = last_strip().lock() else {
        return result;
    };
    commit_strip_state_after_apply(&mut slot, &entries, result)
}

fn commit_strip_state_after_apply(
    current: &mut Vec<StripEntry>,
    next: &[StripEntry],
    result: Result<(), String>,
) -> Result<(), String> {
    result?;
    *current = next.to_vec();
    Ok(())
}

fn strip_is_active(strip_ok: bool, entries: &[StripEntry]) -> bool {
    strip_ok && !entries.is_empty()
}

fn strip_icon_ids_to_clear(known: &[StripEntry], attempted: &[StripEntry]) -> Vec<String> {
    let mut ids: Vec<String> = STRIP_PROVIDER_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect();
    for entry in known.iter().chain(attempted) {
        if !ids.iter().any(|seen| seen == &entry.id) {
            ids.push(entry.id.clone());
        }
    }
    ids
}

#[tauri::command]
async fn sync_tray_surfaces(
    app: tauri::AppHandle,
    revision: u64,
    projection: tray_projection::TrayProjectionConfig,
    entries: Vec<StripEntry>,
) -> Result<(), String> {
    let strip_result = update_tray_strip(app.clone(), entries.clone(), Some(revision)).await;
    let main_result = publish_authorized_main_tray(
        app, Some(revision), Some(projection), strip_is_active(strip_result.is_ok(), &entries),
    ).await;
    match (main_result, strip_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(main_error), Err(strip_error)) => Err(format!("{main_error}; {strip_error}")),
    }
}

fn validate_strip_entries(entries: &[StripEntry]) -> Result<(), String> {
    if entries.len() > 4 {
        return Err("tray strip accepts at most 4 providers".into());
    }
    for (index, entry) in entries.iter().enumerate() {
        if !strip_provider_id_is_allowed(&entry.id) {
            return Err(format!("invalid tray strip provider id: {}", entry.id));
        }
        if entries[..index].iter().any(|seen| seen.id == entry.id) {
            return Err(format!("duplicate tray strip provider id: {}", entry.id));
        }
        if entry.logo.len() != 32 * 32 * 4 {
            return Err(format!("invalid tray strip logo for {}", entry.id));
        }
        if entry.labels.is_empty() || entry.labels.len() > 2 {
            return Err(format!("invalid tray strip values for {}", entry.id));
        }
    }
    Ok(())
}

fn strip_provider_id_is_allowed(id: &str) -> bool {
    match id.split_once('@') {
        None => STRIP_PROVIDER_IDS.contains(&id),
        Some((family, account)) => {
            STRIP_PROVIDER_IDS.contains(&family)
                && !account.is_empty()
                && account
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        }
    }
}

fn strip_tray_key(id: &str) -> String {
    id.replace('@', "--")
}

fn strip_reset_ids(previous: &[StripEntry], next: &[StripEntry]) -> Vec<String> {
    let same_order = previous.len() == next.len()
        && previous.iter().zip(next).all(|(old, new)| old.id == new.id);
    if same_order {
        return Vec::new();
    }

    let mut ids = Vec::new();
    for entry in previous.iter().chain(next) {
        if !ids.contains(&entry.id) {
            ids.push(entry.id.clone());
        }
    }
    ids
}

fn strip_entry_application_order(entries: &[StripEntry], rebuild_order: bool) -> Vec<&StripEntry> {
    let mut ordered: Vec<&StripEntry> = entries.iter().collect();
    if rebuild_order {
        // Windows inserts each new tray icon to the left. Rebuild Provider
        // pairs from right to left so their visible order matches providerOrder.
        ordered.reverse();
    }
    ordered
}

async fn clear_tray_strip_icons(
    app: tauri::AppHandle,
    known: &[StripEntry],
    attempted: &[StripEntry],
) -> Result<(), String> {
    let ids = strip_icon_ids_to_clear(known, attempted);
    let handle = app.clone();
    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    app.run_on_main_thread(move || {
        for id in &ids {
            let key = strip_tray_key(id);
            handle.remove_tray_by_id(&format!("strip-logo-{key}"));
            handle.remove_tray_by_id(&format!("strip-num-{key}"));
        }
        let _ = sender.blocking_send(());
    })
    .map_err(|error| error.to_string())?;
    receiver
        .recv()
        .await
        .ok_or_else(|| "tray strip clear ended before reporting a result".to_string())
}

async fn apply_tray_strip(
    app: tauri::AppHandle,
    entries: Vec<StripEntry>,
    reset_ids: Vec<String>,
    rebuild_order: bool,
    revision: Option<u64>,
) -> Result<(), String> {
    let handle = app.clone();
    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    app.run_on_main_thread(move || {
        let apply = |entries: Vec<StripEntry>| -> Result<(), String> {
            // Removal returns None when an icon is already absent; that is
            // the desired end state rather than an update failure.
            for id in STRIP_PROVIDER_IDS {
                if !entries.iter().any(|entry| entry.id == id) {
                    handle.remove_tray_by_id(&format!("strip-logo-{id}"));
                    handle.remove_tray_by_id(&format!("strip-num-{id}"));
                }
            }
            for id in &reset_ids {
                let key = strip_tray_key(id);
                handle.remove_tray_by_id(&format!("strip-logo-{key}"));
                handle.remove_tray_by_id(&format!("strip-num-{key}"));
            }

            for entry in strip_entry_application_order(&entries, rebuild_order) {
                let tray_key = strip_tray_key(&entry.id);
                let logo_id = format!("strip-logo-{tray_key}");
                let num_id = format!("strip-num-{tray_key}");
                let logo_icon = tauri::image::Image::new_owned(entry.logo.clone(), 32, 32);
                let num_icon = tauri::image::Image::new_owned(
                    draw_tray_numbers(&entry.values),
                    32,
                    32,
                );
                let tooltip = entry.tooltip.clone();

                let new_trays = if let Some(tray) = handle.tray_by_id(&num_id) {
                    tray.set_icon(Some(num_icon))
                        .map_err(|error| format!("set {} strip numbers: {error}", entry.id))?;
                    tray.set_tooltip(Some(&tooltip))
                        .map_err(|error| format!("set {} strip tooltip: {error}", entry.id))?;
                    if let Some(logo_tray) = handle.tray_by_id(&logo_id) {
                        logo_tray.set_tooltip(Some(&tooltip)).map_err(|error| {
                            format!("set {} strip logo tooltip: {error}", entry.id)
                        })?;
                        Vec::new()
                    } else {
                        vec![(logo_id, logo_icon)]
                    }
                } else {
                    vec![(num_id, num_icon), (logo_id, logo_icon)]
                };

                // New pairs are numbers first: Windows inserts each new tray
                // icon to the left, yielding "logo | numbers" on screen.
                for (tray_id, icon) in new_trays {
                    TrayIconBuilder::with_id(tray_id)
                        .icon(icon)
                        .tooltip(&tooltip)
                        .show_menu_on_left_click(false)
                        .on_tray_icon_event(|tray, event| {
                            if let TrayIconEvent::Click {
                                button: MouseButton::Left,
                                button_state: MouseButtonState::Up,
                                position,
                                ..
                            } = event
                            {
                                toggle_popover(tray.app_handle(), position);
                            }
                        })
                        .build(&handle)
                        .map_err(|error| format!("build {} strip icon: {error}", entry.id))?;
                }
            }
            Ok(())
        };
        let result = if let Some(revision) = revision {
            usage_publications().apply(access_runtime(), revision, |snapshots, policy| {
                apply(derive_strip_entries(&entries, snapshots, policy))
            }).unwrap_or_else(|| Err("Stale native strip publication".into()))
        } else { apply(Vec::new()) };
        let _ = sender.blocking_send(result);
    })
    .map_err(|error| error.to_string())?;
    receiver
        .recv()
        .await
        .ok_or_else(|| "tray strip update ended before reporting a result".to_string())?
}

// ---------------------------------------------------------------------------
// Usage fetching
// ---------------------------------------------------------------------------

/// A provider that just failed gets benched briefly instead of being
/// re-probed on every refresh: 60s for ordinary errors, 5 minutes for rate
/// limits (hammering a 429 makes it worse — learned that the hard way).
struct FailState {
    until_ms: i64,
    note: String,
    // Claude's last real failure survives an unrelated publication clear.
    // This map is already cleared by account revocation and mode/cache reset.
    observed_error: Option<providers::Snapshot>,
}

fn fail_state() -> &'static Mutex<HashMap<String, FailState>> {
    static STATE: OnceLock<Mutex<HashMap<String, FailState>>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct CachedSnap {
    at: i64,
    snap: providers::Snapshot,
}

fn last_ok() -> &'static Mutex<HashMap<String, CachedSnap>> {
    static LAST_OK: OnceLock<Mutex<HashMap<String, CachedSnap>>> = OnceLock::new();
    LAST_OK.get_or_init(|| {
        let cache_file = providers::config_dir().join("last_snapshots.json");
        let mut loaded: HashMap<String, CachedSnap> = std::fs::read_to_string(&cache_file)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        loaded.retain(|id, _| !retired_id(id));
        Mutex::new(loaded)
    })
}

fn persist_last_ok_at(
    path: &std::path::Path,
    map: &HashMap<String, CachedSnap>,
) -> Result<(), String> {
    let retained: HashMap<_, _> = map.iter().filter(|(id, _)| !retired_id(id)).collect();
    let serialized =
        serde_json::to_string(&retained).map_err(|e| format!("serialize snapshot cache: {e}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| "snapshot cache path has no parent".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create snapshot cache dir: {e}"))?;
    std::fs::write(path, serialized).map_err(|e| format!("write snapshot cache: {e}"))
}

// Thread-local, not global: a process-wide one-shot flag gets stolen by
// whichever parallel test calls persist_last_ok inside the injecting
// test's store -> consume window, failing BOTH tests at once.
#[cfg(test)]
thread_local! {
    static TEST_PERSIST_LAST_OK_FAIL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
static SNAPSHOT_CACHE_NEEDS_FLUSH: AtomicBool = AtomicBool::new(false);

fn persist_last_ok(map: &HashMap<String, CachedSnap>) -> Result<(), String> {
    #[cfg(test)]
    if TEST_PERSIST_LAST_OK_FAIL.with(|fail| fail.replace(false)) {
        SNAPSHOT_CACHE_NEEDS_FLUSH.store(true, Ordering::Release);
        return Err("test: persist last_ok failed".into());
    }
    if cfg!(test) {
        SNAPSHOT_CACHE_NEEDS_FLUSH.store(false, Ordering::Release);
        return Ok(());
    }
    let cache_file = providers::config_dir().join("last_snapshots.json");
    let result = persist_last_ok_at(&cache_file, map);
    SNAPSHOT_CACHE_NEEDS_FLUSH.store(result.is_err(), Ordering::Release);
    result
}

fn forget_provider_snapshots(ids: &[String]) -> Result<(), String> {
    forget_provider_snapshots_inner(ids, false)
}

/// Drop cached snapshots for `ids`. Memory, fail-state, alerts, and the
/// local HTTP publication are always cleared so a failed disk write cannot
/// keep serving the old account. `persist_even_if_unchanged` rewrites the
/// cache file on retry after a persist failure (memory may already be clean).
fn forget_provider_snapshots_inner(
    ids: &[String],
    persist_even_if_unchanged: bool,
) -> Result<(), String> {
    let mut map = last_ok().lock().unwrap();
    let mut next = map.clone();
    let mut changed = false;
    for id in ids {
        changed |= next.remove(id).is_some();
    }
    *map = next.clone();
    drop(map);
    let mut failures = fail_state().lock().unwrap();
    for id in ids {
        failures.remove(id);
        alerts::forget_snapshot(id);
    }
    httpapi::forget_snapshots(ids);
    if changed || persist_even_if_unchanged {
        persist_last_ok(&next)?;
    }
    Ok(())
}

fn forget_provider_snapshot(id: &str) -> Result<(), String> {
    forget_provider_snapshots(&[id.to_string()])
}















fn retain_current_key_card_results(
    all: &mut Vec<providers::Snapshot>,
    expected: &HashMap<String, u64>,
    current: &HashMap<String, u64>,
) -> Vec<String> {
    let stale: Vec<String> = all
        .iter()
        .filter(|snapshot| {
            is_credential_scoped_card(&snapshot.id)
                && expected.get(&snapshot.id) != current.get(&snapshot.id)
        })
        .map(|snapshot| snapshot.id.clone())
        .collect();
    let stale_set: HashSet<&str> = stale.iter().map(String::as_str).collect();
    all.retain(|snapshot| !stale_set.contains(snapshot.id.as_str()));
    stale
}

/// The "current" side of the generation check: one map covering every
/// credential-scoped snapshot in the batch. Built from the same id
/// universe as the expected side — if this ever narrows back to managed
/// cards only, every plain provider's `Some(0)` would compare unequal to
/// a missing entry and each refresh would silently drop its results.
fn current_credential_scoped_generations(all: &[providers::Snapshot]) -> HashMap<String, u64> {
    key_card_snapshot_generations(
        all.iter()
            .filter(|snapshot| is_credential_scoped_card(&snapshot.id))
            .map(|snapshot| snapshot.id.clone()),
    )
}

static KEY_CARD_MUTATION_GENERATION: AtomicU64 = AtomicU64::new(0);
static KEY_CARD_ACTIVE_MUTATIONS: AtomicU64 = AtomicU64::new(0);
static KEY_CARD_SNAPSHOT_GENERATIONS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
// Serialize only cache/publication and local mutations, never network requests.
static KEY_CARD_PUBLICATION: Mutex<()> = Mutex::new(());

fn key_card_mutation_generation() -> u64 {
    KEY_CARD_MUTATION_GENERATION.load(Ordering::Acquire)
}

fn key_card_snapshot_generations(ids: impl IntoIterator<Item = String>) -> HashMap<String, u64> {
    let generations = KEY_CARD_SNAPSHOT_GENERATIONS
        .get_or_init(Default::default)
        .lock()
        .unwrap();
    ids.into_iter()
        .map(|id| {
            let generation = generations.get(&id).copied().unwrap_or(0);
            (id, generation)
        })
        .collect()
}

fn bump_key_card_snapshot_generations(ids: &[String]) {
    let mut generations = KEY_CARD_SNAPSHOT_GENERATIONS
        .get_or_init(Default::default)
        .lock()
        .unwrap();
    for id in ids {
        *generations.entry(id.clone()).or_default() += 1;
    }
}

struct KeyCardMutationGuard {
    snapshot_ids: Vec<String>,
    _publication: std::sync::MutexGuard<'static, ()>,
}

impl KeyCardMutationGuard {
    fn begin(snapshot_ids: Vec<String>) -> Self {
        let publication = KEY_CARD_PUBLICATION.lock().unwrap_or_else(|e| e.into_inner());
        KEY_CARD_ACTIVE_MUTATIONS.fetch_add(1, Ordering::AcqRel);
        bump_key_card_snapshot_generations(&snapshot_ids);
        KEY_CARD_MUTATION_GENERATION.fetch_add(1, Ordering::AcqRel);
        Self { snapshot_ids, _publication: publication }
    }

    fn track(&mut self, snapshot_ids: Vec<String>) {
        bump_key_card_snapshot_generations(&snapshot_ids);
        self.snapshot_ids.extend(snapshot_ids);
    }
}

impl Drop for KeyCardMutationGuard {
    fn drop(&mut self) {
        bump_key_card_snapshot_generations(&self.snapshot_ids);
        KEY_CARD_MUTATION_GENERATION.fetch_add(1, Ordering::AcqRel);
        KEY_CARD_ACTIVE_MUTATIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Strip deleted key cards from config, preserving their family choices.
/// Returns only changed config fields.
fn purge_key_cards_from_config(cfg: &mut Value, snapshot_ids: &[String]) -> Value {
    if snapshot_ids.is_empty() {
        return json!({});
    }
    let drop: HashSet<&str> = snapshot_ids.iter().map(String::as_str).collect();
    let mut patch = serde_json::Map::new();

    if let Some(arr) = cfg.get_mut("disabled").and_then(Value::as_array_mut) {
        let before = arr.len();
        arr.retain(|v| v.as_str().map(|s| !drop.contains(s)).unwrap_or(true));
        if arr.len() != before {
            patch.insert("disabled".into(), Value::Array(arr.clone()));
        }
    }

    let mut layout_changed = false;
    if let Some(layout) = cfg.get_mut("layout").and_then(Value::as_object_mut) {
        if let Some(order) = layout
            .get_mut("providerOrder")
            .and_then(Value::as_array_mut)
        {
            let before = order.len();
            order.retain(|v| v.as_str().map(|s| !drop.contains(s)).unwrap_or(true));
            layout_changed |= order.len() != before;
        }
        if let Some(providers) = layout.get_mut("providers").and_then(Value::as_object_mut) {
            for id in snapshot_ids {
                layout_changed |= providers.remove(id).is_some();
            }
        }
    }
    if layout_changed {
        if let Some(layout) = cfg.get("layout") {
            patch.insert("layout".into(), layout.clone());
        }
    }

    let pinned_hit = cfg
        .get("pinned")
        .and_then(|p| p.get("provider"))
        .and_then(Value::as_str)
        .is_some_and(|p| drop.contains(p));
    if pinned_hit {
        cfg["pinned"] = Value::Null;
        patch.insert("pinned".into(), Value::Null);
    }

    if let Some(arr) = cfg.get_mut("trayProviders").and_then(Value::as_array_mut) {
        let before = arr.len();
        arr.retain(|v| v.as_str().map(|s| !drop.contains(s)).unwrap_or(true));
        if arr.len() != before {
            patch.insert("trayProviders".into(), Value::Array(arr.clone()));
        }
    }

    Value::Object(patch)
}

fn key_cards_purge_restore_patch(original: &Value, purge_patch: &Value) -> Value {
    let mut restore = serde_json::Map::new();
    if let Some(obj) = purge_patch.as_object() {
        for key in obj.keys() {
            restore.insert(key.clone(), original.get(key).cloned().unwrap_or(Value::Null));
        }
    }
    Value::Object(restore)
}

fn persist_key_cards_config_purge(snapshot_ids: &[String]) -> Result<Value, String> {
    // Tests must not rewrite the developer's real config.json.
    if cfg!(test) {
        return Ok(json!({}));
    }
    let mut cfg = config_with_defaults(load_config());
    let original = cfg.clone();
    let patch = purge_key_cards_from_config(&mut cfg, snapshot_ids);
    let restore = key_cards_purge_restore_patch(&original, &patch);
    if patch.as_object().is_some_and(|o| !o.is_empty()) {
        set_config_inner(patch)?;
    }
    Ok(restore)
}

fn restore_key_cards_config_purge(restore: Value) -> Result<(), String> {
    if restore.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        return Ok(());
    }
    if cfg!(test) {
        return Ok(());
    }
    set_config_inner(restore).map(|_| ())
}









fn rename_cached_snapshot(id: &str, new_name: String) -> Result<(), String> {
    rename_cached_snapshots(&[(id.to_string(), new_name)])
}

fn rename_cached_snapshots(renames: &[(String, String)]) -> Result<(), String> {
    let mut map = last_ok().lock().unwrap();
    rename_cached_snapshots_in(&mut map, renames, persist_last_ok)?;
    httpapi::rename_snapshots(&renames.iter().cloned().collect());
    Ok(())
}

#[cfg(test)]
fn rename_cached_snapshot_in<Persist>(
    map: &mut HashMap<String, CachedSnap>,
    id: &str,
    new_name: String,
    persist: Persist,
) -> Result<(), String>
where
    Persist: FnOnce(&HashMap<String, CachedSnap>) -> Result<(), String>,
{
    rename_cached_snapshots_in(map, &[(id.to_string(), new_name)], persist)
}

fn rename_cached_snapshots_in<Persist>(
    map: &mut HashMap<String, CachedSnap>,
    renames: &[(String, String)],
    persist: Persist,
) -> Result<(), String>
where
    Persist: FnOnce(&HashMap<String, CachedSnap>) -> Result<(), String>,
{
    let mut next = map.clone();
    let mut changed = false;
    for (id, new_name) in renames {
        if let Some(entry) = next.get_mut(id) {
            if entry.snap.name != *new_name {
                entry.snap.name = new_name.clone();
                changed = true;
            }
        }
    }
    if changed {
        persist(&next)?;
        *map = next;
    }
    Ok(())
}

/// The provider family of a card id: "claude@ab12cd34" → "claude". The only
/// spelling allowed to leave the machine in telemetry.
fn family_of(id: &str) -> String {
    id.split('@').next().unwrap_or(id).to_string()
}





fn is_managed_key_card(id: &str) -> bool {
    matches!(family_of(id).as_str(), "onenewapi" | "sub2api")
}

/// The plain API-key providers set_api_key accepts, in
/// %APPDATA%\Pane\<provider>.json. Single source of truth for both the
/// save command's validation and the credential-context bookkeeping below.
const API_KEY_PROVIDERS: &[&str] = &[
    "openrouter",
    "zai",
    "commandcode",
    "minimax",
    "deepseek",
    "moonshot",
    "kimi",
    "elevenlabs",
    "codebuff",
    "kilo",
    "aihubmix",
    "qwen",
    "stepfun",
];

fn is_plain_api_key_provider(family: &str) -> bool {
    API_KEY_PROVIDERS.contains(&family)
}

/// Cards whose cached snapshots, cooldowns, and alerts belong to one
/// specific credential and must be dropped when that credential changes:
/// managed key cards plus the plain API-key providers. Everything else
/// (CLI-login families like claude/codex) is handled by the separate
/// cache-identity stamp, not by generations.
fn is_credential_scoped_card(id: &str) -> bool {
    let family = family_of(id);
    is_managed_key_card(id) || is_plain_api_key_provider(&family)
}

/// Snapshots to drop when this pasted key changes. Moonshot's wallet
/// folds into the Kimi card, so rotating Moonshot must also forget the
/// Kimi snapshot. Rotating only Kimi leaves the Moonshot snapshot —
/// that wallet key did not change.
fn api_key_snapshot_ids(provider: &str) -> Vec<String> {
    match provider {
        "moonshot" => vec!["moonshot".into(), "kimi".into()],
        _ => vec![provider.to_string()],
    }
}

/// Credit high-water marks belong to one pasted key. Kimi and Moonshot
/// do not share a pot — rotating Kimi must not zero Moonshot's meter.
fn api_key_baseline_ids(provider: &str) -> Vec<String> {
    vec![provider.to_string()]
}

/// Managed API families disable all their key cards together.
/// Claude/Codex extra accounts stay independent of the bare family id.
fn card_is_disabled(id: &str, disabled: &[String]) -> bool {
    if disabled.iter().any(|d| d == id) {
        return true;
    }
    is_managed_key_card(id) && disabled.iter().any(|d| d == &family_of(id))
}

// Owned id/name so dynamically discovered account cards (claude@<hash>)
// can ride the same guard as the static providers under a 'static spawn.
async fn guarded<F>(id: String, name: String, fut: F) -> providers::Snapshot
where
    F: std::future::Future<Output = providers::Snapshot>,
{
    let id = id.as_str();
    let name = name.as_str();
    if let Err(error) = access_policy::check_current_operation() {
        return providers::Snapshot::error(id, name, error);
    }
    // A credential rotation bumps this card's generation under the
    // publication lock. Capturing it before the request and comparing after
    // means a result that outlived its own key neither benches nor unbenches
    // the replacement's fail state (fetch_usage drops it separately).
    let expected_generation = key_card_snapshot_generations([id.to_string()])
        .get(id)
        .copied()
        .unwrap_or(0);
    let _credit_bind = CreditMeterBindGuard::begin(credit_meter_bind_ids(id));
    let now = now_ms() as i64;
    let benched = {
        let map = fail_state().lock().unwrap();
        map.get(id)
            .filter(|f| now < f.until_ms)
            .map(|f| (f.note.clone(), f.observed_error.as_ref().and_then(|s| s.attempted_at)))
    };
    if let Some((note, attempted_at)) = benched {
        let mut snap = providers::Snapshot::error(id, name, note);
        snap.attempted_at = attempted_at;
        return snap;
    }
    let mut snap = fut.await;
    // Each actual attempt has its own clock; scoped merges cannot use the
    // retained publication time for a newly queried error or no-credential row.
    snap.attempted_at = Some(now_ms() as i64);
    if access_policy::check_current_operation().is_err() { return snap; }
    let generation_now = key_card_snapshot_generations([id.to_string()])
        .get(id)
        .copied()
        .unwrap_or(0);
    if generation_now != expected_generation {
        return snap;
    }
    let mut map = fail_state().lock().unwrap();
    let metadata_failure = (id == "commandcode" && snap.status == "ok")
        .then(|| snap.warning.clone()).flatten().filter(|warning| warning.contains("HTTP "));
    if snap.status == "error" || metadata_failure.is_some() {
        let err = metadata_failure.or_else(|| snap.error.clone()).unwrap_or_default();
        let rate_limited = err.contains("429");
        // A vendor-stated Retry-After wins over our fixed backoff — bench
        // for exactly that long (capped at an hour) instead of knocking on
        // a door the server said stays shut.
        let retry_after_ms = err
            .split("retry_after_s=")
            .nth(1)
            .and_then(|rest| {
                rest.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<i64>()
                    .ok()
            })
            .map(|s| (s * 1000).min(3_600_000));
        let bench_ms = retry_after_ms.unwrap_or(if rate_limited { 300_000 } else { 60_000 });
        map.insert(
            id.to_string(),
            FailState {
                until_ms: now + bench_ms,
                observed_error: Some(if snap.status == "ok" {
                    let mut error = providers::Snapshot::error(id, name, err.clone());
                    error.attempted_at = snap.attempted_at;
                    error
                } else { snap.clone() }),
                note: if let Some(ms) = retry_after_ms {
                    format!(
                        "rate limited — the vendor asked to wait ~{}m",
                        (ms / 60_000).max(1)
                    )
                } else if rate_limited {
                    format!("rate limited — cooling down for a few minutes ({err})")
                } else {
                    err
                },
            },
        );
    } else {
        map.remove(id);
    }
    snap
}

/// Reuse the Moonshot account guard for the folded wallet. The returned attempt
/// clock belongs to its real operation; a cooldown replay keeps that clock.
async fn guarded_moonshot_wallet() -> providers::Snapshot {
    guarded("moonshot".into(), "Kimi API".into(), async {
        match providers::moonshot::api_rows().await {
            Ok(rows) => providers::Snapshot::ok("moonshot", "Kimi API", None, rows),
            Err(error) => providers::Snapshot::error("moonshot", "Kimi API", error),
        }
    }).await
}

/// Last-good Kimi snapshot on disk. Used to skip the leftover Moonshot
/// fetch only when that card has actually painted *recently* — a
/// credentials file, or a day-old cache entry, must not hide the wallet.
fn cached_kimi_ok() -> bool {
    let path = providers::config_dir().join("last_snapshots.json");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(doc) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    cached_kimi_ok_from(&doc, now_ms)
}

fn cached_kimi_ok_from(doc: &Value, now_ms: i64) -> bool {
    if doc.pointer("/kimi/snap/status").and_then(Value::as_str) != Some("ok") {
        return false;
    }
    let at = doc.pointer("/kimi/at").and_then(Value::as_i64).unwrap_or(0);
    at > 0 && now_ms.saturating_sub(at) <= SNAPSHOT_CACHE_MS
}

fn fold_moonshot_into_kimi(all: &mut Vec<providers::Snapshot>) {
    let Some(kimi) = all.iter().find(|s| s.id == "kimi" && s.status == "ok") else {
        return;
    };
    // Don't throw away a freshly fetched wallet just because the plan
    // card loaded. Fold only when Kimi already carries those rows, or
    // when Moonshot has nothing to show (plan-only / no_credentials).
    let kimi_has_wallet = kimi.metrics.iter().any(|m| is_kimi_wallet_label(&m.label));
    let moonshot_has_rows = all
        .iter()
        .any(|s| s.id == "moonshot" && !s.metrics.is_empty());
    if kimi_has_wallet || !moonshot_has_rows {
        all.retain(|s| s.id != "moonshot");
    }
}

fn is_kimi_wallet_label(label: &str) -> bool {
    matches!(
        label,
        "API" | "Credits used" | "Balance" | "Vouchers" | "Cash"
    )
}

fn restore_kimi_wallet_rows(current: &mut providers::Snapshot, previous: &providers::Snapshot) {
    if current.metrics.iter().any(|m| m.label == "API") {
        return;
    }
    for m in &previous.metrics {
        if is_kimi_wallet_label(&m.label) && !current.metrics.iter().any(|x| x.label == m.label) {
            current.metrics.push(m.clone());
        }
    }
}

/// A scoped Kimi query only refreshes the plan. Preserve the last displayed
/// wallet, including its independent error and clock, without inventing rows.
fn preserve_unqueried_kimi_wallet(current: &mut providers::Snapshot, previous: &providers::Snapshot) {
    current.metrics.retain(|m| !is_kimi_wallet_label(&m.label));
    current.metrics.extend(previous.metrics.iter().filter(|m| is_kimi_wallet_label(&m.label)).cloned());
    // A first-ever failed wallet query has provenance/error but no values yet.
    // A plan-only refresh must preserve that history without inventing rows.
    if previous.wallet_history.is_some() || current.metrics.iter().any(|m| is_kimi_wallet_label(&m.label)) {
        current.wallet_history = Some(previous.wallet_history.clone().unwrap_or_else(|| snapshot::WalletHistory {
            source_account_id: "moonshot".into(),
            fetched_at: previous.fetched_at,
            attempted_at: previous.attempted_at,
            warning: previous.warning.clone().filter(|warning| warning.starts_with("Moonshot API wallet")),
        }));
    }
}

fn restore_last_success_after_error(
    current: &mut providers::Snapshot,
    previous: &providers::Snapshot,
    age_ms: i64,
) -> bool {
    let keep_manual_history = matches!(family_of(&current.id).as_str(), "sub2api" | "claude" | "commandcode");
    if current.status != "error" || (!keep_manual_history && age_ms > SNAPSHOT_CACHE_MS) {
        return false;
    }
    let warning = current.error.clone();
    let attempted_at = current.attempted_at;
    *current = previous.clone();
    current.attempted_at = attempted_at;
    current.attempt_failed = true;
    if keep_manual_history || age_ms > STALE_GRACE_MS {
        current.stale = true;
        current.warning = warning;
    }
    true
}

/// Old `last_snapshots.json` entries have no `fetched_at` on the snap
/// itself. The cache clock (`CachedSnap.at`) is the last success time.
fn hydrate_fetch_time(s: &mut providers::Snapshot, at: i64) {
    if s.fetched_at.is_none() {
        s.fetched_at = Some(at);
    }
}

/// A forced repaint/settings refresh is not evidence of a user query. Omitted
/// reasons remain automatic, including older callers of this command.
#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum UsageRefreshReason {
    #[default]
    Automatic,
    UserRefresh,
}

/// Query scope is independent from the complete backend authorization policy.
/// An exact account selector never means all accounts in a provider family.
#[derive(Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum UsageRefreshScope {
    All {},
    Account { #[serde(rename = "accountId")] account_id: String },
}

fn usage_refresh_gate() -> &'static tokio::sync::Mutex<()> {
    static GATE: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    GATE.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Synchronous identity reads obey the current scheduler setting too. An
/// automatic query admitted before a settings save may finish, but publication
/// must not open another credential source after its gate has closed.
fn read_usage_identity<T>(policy: &access_policy::AccessPolicy, automatic: bool,
    id: &str, read: impl FnOnce() -> T) -> Option<T> {
    if automatic && !provider_auto_refresh::enabled(&load_config(), id) { return None; }
    policy.read_account(id, read)
}

/// Render remembered manual-only accounts without consulting their credential or
/// identity sources. These are historical results, not verified current-login
/// readings. Preserve a real last-attempt error from the current publication;
/// otherwise use the last-success cache without advancing its clock.
fn append_manual_usage_snapshots(
    all: &mut Vec<providers::Snapshot>,
    policy: &access_policy::AccessPolicy,
    disabled: &[String],
    cache: &HashMap<String, CachedSnap>,
    cfg: &Value,
) {
    let previous = usage_publications().read(access_runtime());
    for id in policy.enabled_accounts.iter().filter(|id| {
        !provider_auto_refresh::enabled(cfg, id) && id.as_str() != "hermes" && policy.allows_account(id) && !card_is_disabled(id, disabled)
    }) {
        if all.iter().any(|s| &s.id == id) { continue; }
        let last_attempt = previous.as_ref()
            .and_then(|batch| batch.snapshots.iter().find(|s| &s.id == id && s.status != "manual"));
        let observed_error = fail_state().lock().unwrap_or_else(|e| e.into_inner())
            .get(id).and_then(|failure| failure.observed_error.clone());
        let mut snap = if let Some(mut error) = observed_error {
            if let Some(cached) = cache.get(id) {
                if restore_last_success_after_error(&mut error, &cached.snap, now_ms() as i64 - cached.at) {
                    hydrate_fetch_time(&mut error, cached.at);
                }
            }
            error
        } else if let Some(snap) = last_attempt {
            snap.clone()
        } else if let Some(cached) = cache.get(id) {
            let mut snap = cached.snap.clone();
            hydrate_fetch_time(&mut snap, cached.at);
            snap
        } else {
            let name = policy.account_bindings.get(id).map(|binding| binding.name.as_str()).unwrap_or_else(|| match id.as_str() {
                "claude" => "Claude",
                "codex" => "Codex",
                "cursor" => "Cursor",
                "opencode" => "OpenCode",
                "copilot" => "Copilot",
                "grok" => "Grok",
                "devin" => "Devin",
                "minimax" => "MiniMax",
                "openrouter" => "OpenRouter",
                "zai" => "Z.ai",
                "commandcode" => "CommandCode",
                "antigravity" => "Antigravity",
                "deepseek" => "DeepSeek",
                "moonshot" => "Kimi API",
                "elevenlabs" => "ElevenLabs",
                "ollama" => "Ollama",
                "codebuff" => "Codebuff",
                "kilo" => "Kilo",
                "aihubmix" => "AihubMix",
                "qwen" => "Qwen Code",
                "hermes" => "Hermes",
                "kimi" => "Kimi Code",
                "stepfun" => "StepFun",
                _ => id,
            });
            let mut snap = providers::Snapshot::ok(id, name, None, vec![]);
            snap.status = "manual".into();
            snap
        };
        if snap.status == "ok" { snap.stale = true; }
        all.push(snap);
    }
}

/// Automatic refresh follows per-family scheduler policy before any provider
/// reads. Explicit manual intent bypasses only that policy, never consent.
/// A persisted setting change does not revoke in-flight work or clear history.
#[tauri::command]
async fn fetch_usage(
    app: tauri::AppHandle,
    disabled: Option<Vec<String>>,
    reason: Option<UsageRefreshReason>,
    scope: Option<UsageRefreshScope>,
) -> Result<usage_publication::UsageResult, String> {
    // Serialize complete refresh transactions, including scope validation. A
    // slow full pass cannot overwrite a newer targeted result at publication.
    let _refresh = usage_refresh_gate().lock().await;
    let cfg = config_with_defaults(load_config());
    let (full_policy, access_revision) = access_runtime().snapshot();
    let mut policy = full_policy.clone();
    let target = match scope.unwrap_or(UsageRefreshScope::All {}) {
        UsageRefreshScope::All {} => None,
        UsageRefreshScope::Account { account_id } => Some(account_id),
    };
    let mut effective_disabled: Vec<String> = cfg.get("disabled")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    effective_disabled.extend(disabled.unwrap_or_default());
    effective_disabled.sort();
    effective_disabled.dedup();
    if let Some(id) = target.as_deref() {
        if !full_policy.known_account(id) || !full_policy.allows_account(id)
            || card_is_disabled(id, &effective_disabled)
        {
            return Err("Refresh target is unknown or disabled".into());
        }
        // Propagate exact query narrowing to nested account permits, including
        // Kimi's separately authorized Moonshot wallet. This is not a revocation.
        effective_disabled.extend(access_policy::FAMILIES.iter()
            .map(|id| (*id).to_string()).chain(full_policy.account_bindings.keys().cloned())
            .filter(|other| other != id));
        effective_disabled.sort();
        effective_disabled.dedup();
    }
    let disabled = effective_disabled;

    policy.enabled_accounts.retain(|id| !disabled.contains(id)
        && target.as_ref().is_none_or(|target| id == target));
    let publication_policy = if target.is_some() { &full_policy } else { &policy };
    let automatic = matches!(reason.unwrap_or_default(), UsageRefreshReason::Automatic);
    let mut query_policy = policy.clone();
    if automatic {
        query_policy.enabled_accounts.retain(|id| id == "hermes" || provider_auto_refresh::enabled(&cfg, id));
    }
    // Nested account permits inherit this query narrowing, so Kimi cannot use
    // its own automatic permission to query an auto-disabled Moonshot wallet.
    let mut query_disabled = disabled.clone();
    query_disabled.extend(full_policy.enabled_accounts.iter()
        .filter(|id| !query_policy.allows_account(id)).cloned());
    let query_claude = query_policy.enabled_accounts.iter()
        .any(|id| family_of(id) == "claude" && query_policy.allows_account(id));
    let query_commandcode = query_policy.allows_account("commandcode");

    let commandcode_identity_at_start = if query_commandcode && (!automatic || provider_auto_refresh::enabled(&load_config(), "commandcode")) {
        let identity = access_runtime().commit_if_current(access_revision, |current| {
            current.read_account("commandcode", providers::commandcode::default_identity).flatten()
        }).flatten();
        let stored: Value = std::fs::read_to_string(providers::config_dir().join("cache_identities.json"))
            .ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or(Value::Null);
        if stored.get("commandcode").and_then(Value::as_str) != identity.as_deref() {
            // A new local key must not inherit the previous key's backoff.
            fail_state().lock().unwrap_or_else(|e| e.into_inner()).remove("commandcode");
        }
        identity
    } else { None };

    // Bind the default OpenCode fingerprint to this refresh so a swap of
    // auth.json while the request is in flight cannot cache the old key's
    // numbers under the new identity.
    let opencode_identity_at_start = read_usage_identity(&query_policy, automatic, "opencode", providers::opencode::default_identity).flatten();

    // Same guard for the capacity families: a default Claude/Codex
    // sign-in swap while the requests are in flight must not write
    // account A's usage poll into account B's quota ledger.

    // Each provider future is boxed onto the heap and spawned as its own
    // task. A single tokio::join! over 28 inlined futures builds one huge
    // combined state machine on the calling thread's stack — at 28 providers
    // that overflowed the main thread's 1 MB stack and killed the app.
    type BoxedSnap =
        std::pin::Pin<Box<dyn std::future::Future<Output = providers::Snapshot> + Send>>;
    // Disabled providers are skipped BEFORE anything is spawned — a merely
    // post-filtered provider still did all its work invisibly: network
    // calls, file reads, and in Kiro's case spawning a CLI whose own
    // auto-updater downloaded a fresh installer to %TEMP% on every refresh
    // (gigabytes within days). Futures are lazy, so building and dropping
    // a disabled entry here runs none of its code.
    let base: Vec<(&str, BoxedSnap)> = vec![
        (
            "claude",
            Box::pin(guarded(
                "claude".into(),
                "Claude".into(),
                providers::claude::snapshot(),
            )),
        ),
        (
            "codex",
            Box::pin(guarded(
                "codex".into(),
                "Codex".into(),
                providers::codex::snapshot(),
            )),
        ),
        (
            "cursor",
            Box::pin(guarded(
                "cursor".into(),
                "Cursor".into(),
                providers::cursor::snapshot(),
            )),
        ),
        (
            "opencode",
            Box::pin(guarded(
                "opencode".into(),
                "OpenCode".into(),
                providers::opencode::snapshot(),
            )),
        ),
        (
            "copilot",
            Box::pin(guarded(
                "copilot".into(),
                "Copilot".into(),
                providers::copilot::snapshot(),
            )),
        ),
        (
            "grok",
            Box::pin(guarded(
                "grok".into(),
                "Grok".into(),
                providers::grok::snapshot(),
            )),
        ),
        (
            "devin",
            Box::pin(guarded(
                "devin".into(),
                "Devin".into(),
                providers::devin::snapshot(),
            )),
        ),
        (
            "minimax",
            Box::pin(guarded(
                "minimax".into(),
                "MiniMax".into(),
                providers::minimax::snapshot(),
            )),
        ),
        (
            "openrouter",
            Box::pin(guarded(
                "openrouter".into(),
                "OpenRouter".into(),
                providers::openrouter::snapshot(),
            )),
        ),
        (
            "zai",
            Box::pin(guarded(
                "zai".into(),
                "Z.ai".into(),
                providers::zai::snapshot(),
            )),
        ),
        (
            "commandcode",
            Box::pin(guarded("commandcode".into(), "CommandCode".into(), providers::commandcode::snapshot())),
        ),
        (
            "antigravity",
            Box::pin(guarded(
                "antigravity".into(),
                "Antigravity".into(),
                providers::antigravity::snapshot(),
            )),
        ),
        (
            "deepseek",
            Box::pin(guarded(
                "deepseek".into(),
                "DeepSeek".into(),
                providers::deepseek::snapshot(),
            )),
        ),
        (
            "moonshot",
            Box::pin(guarded(
                "moonshot".into(),
                "Kimi API".into(),
                providers::moonshot::snapshot(),
            )),
        ),
        (
            "elevenlabs",
            Box::pin(guarded(
                "elevenlabs".into(),
                "ElevenLabs".into(),
                providers::elevenlabs::snapshot(),
            )),
        ),
        (
            "ollama",
            Box::pin(guarded(
                "ollama".into(),
                "Ollama".into(),
                providers::ollama::snapshot(),
            )),
        ),
        (
            "codebuff",
            Box::pin(guarded(
                "codebuff".into(),
                "Codebuff".into(),
                providers::codebuff::snapshot(),
            )),
        ),
        (
            "kilo",
            Box::pin(guarded(
                "kilo".into(),
                "Kilo".into(),
                providers::kilo::snapshot(),
            )),
        ),
        (
            "aihubmix",
            Box::pin(guarded(
                "aihubmix".into(),
                "AihubMix".into(),
                providers::aihubmix::snapshot(),
            )),
        ),
        (
            "qwen",
            Box::pin(guarded(
                "qwen".into(),
                "Qwen Code".into(),
                providers::qwen::snapshot(),
            )),
        ),
        (
            "hermes",
            Box::pin(guarded(
                "hermes".into(),
                "Hermes".into(),
                providers::hermes::snapshot(),
            )),
        ),
        (
            "kimi",
            Box::pin(guarded(
                "kimi".into(),
                "Kimi Code".into(),
                providers::kimi::snapshot(),
            )),
        ),
        (
            "stepfun",
            Box::pin(guarded(
                "stepfun".into(),
                "StepFun".into(),
                providers::stepfun::snapshot(),
            )),
        ),
    ];
    // Skip the leftover Moonshot fetch only when the last Kimi card
    // actually painted — a credentials file alone is not enough (expired
    // login / network blip would otherwise hide the wallet with nothing
    // to fall back to). The post-fetch retain still drops it whenever
    // this cycle's Kimi snapshot is ok.
    let kimi_card_live = target.is_none() && query_policy.allows_account("kimi") && cached_kimi_ok();
    let mut futs: Vec<(String, BoxedSnap)> = base
        .into_iter()
        .filter(|(id, _)| query_policy.allows_account(id) && !card_is_disabled(id, &disabled))
        .filter(|(id, _)| {
            target.is_some()
                || *id != "moonshot"
                || !query_policy.allows_account("kimi")
                || !providers::kimi::has_credentials()
                || disabled.iter().any(|d| d == "kimi")
                || !kimi_card_live
        })
        .map(|(id, fut)| (id.to_string(), fut))
        .collect();
    // Extra Claude accounts (multi-login machines): each discovered config
    // dir renders its own card under a claude@<hash8> id, running the same
    // provider flow scoped to its dir. The default login keeps the bare id.
    for acct in if query_claude && target.as_ref().is_none_or(|id| id.starts_with("claude@")) { providers::claude::discover_extra_accounts_for(&query_policy) } else { Vec::new() } {
        let (id, name, dir) = (acct.id, acct.name, acct.dir);
        futs.push((
            id.clone(),
            Box::pin(guarded(
                id.clone(),
                name.clone(),
                providers::claude::snapshot_at(dir, id, name),
            )),
        ));
    }
    for acct in if query_policy.enabled_accounts.iter().any(|id| id.starts_with("codex@")) && target.as_ref().is_none_or(|id| id.starts_with("codex@")) { providers::codex::discover_extra_accounts_for(&query_policy) } else { Vec::new() } {
        let (id, name, dir) = (acct.id, acct.name, acct.dir);
        futs.push((
            id.clone(),
            Box::pin(guarded(
                id.clone(),
                name.clone(),
                providers::codex::snapshot_at(dir, id, name),
            )),
        ));
    }
    for acct in if query_policy.enabled_accounts.iter().any(|id| id.starts_with("opencode@")) && target.as_ref().is_none_or(|id| id.starts_with("opencode@")) { providers::opencode::discover_extra_accounts_for(&query_policy) } else { Vec::new() } {
        let (id, name, dir, fp) = (acct.id, acct.name, acct.dir, acct.fingerprint);
        futs.push((
            id.clone(),
            Box::pin(guarded(
                id.clone(),
                name.clone(),
                providers::opencode::snapshot_at(dir, id, name, Some(fp)),
            )),
        ));
    }
    // Plain API-key providers ride the same generation scheme as managed
    // key cards: set_api_key bumps a provider's generation when its stored
    // credential actually changes, so a request the old key started can be
    // refused at every write-back below (cache, cooldown, publication).
    let mut expected_key_card_generations = key_card_snapshot_generations(
        futs.iter()
            .map(|(id, _)| id.clone())
            .filter(|id| is_credential_scoped_card(id) && !is_managed_key_card(id))
            .chain(policy.enabled_accounts.iter().filter(|id| is_credential_scoped_card(id)).cloned()),
    );




    let futs: Vec<(String, BoxedSnap)> = futs
        .into_iter()
        .filter(|(id, _)| query_policy.allows_account(id)
                && !card_is_disabled(id, &disabled))
        .collect();
    let missing_target = target.as_ref().filter(|id|
        query_policy.allows_account(id) && !futs.iter().any(|(candidate, _)| candidate == *id)
    ).cloned();
    // Telemetry never learns account-scoped ids — a claude@<hash8> would
    // carry an account-derived hash off the machine. Report families,
    // deduplicated, so a multi-account install looks like "claude" once.
    // (family_of is applied at EVERY telemetry boundary: enabled ids here,
    // refresh outcomes, and starred-metric prefixes.)
    let mut enabled_ids: Vec<String> = {
        let mut fams: Vec<String> = Vec::new();
        for (id, _) in &futs {
            let fam = family_of(id);
            if fam != "sub2api" && !fams.contains(&fam) {
                fams.push(fam);
            }
        }
        fams
    };
    let handles: Vec<_> = futs
        .into_iter()
        .map(|(id, fut)| {
            let runtime = access_runtime().clone();
            let disabled = query_disabled.clone();
            tauri::async_runtime::spawn(async move {
                let check: Option<access_policy::OperationCheck> = automatic.then(|| {
                    Arc::new(|account: &str| account == "hermes" || provider_auto_refresh::enabled(&load_config(), account)) as access_policy::OperationCheck
                });
                runtime.run_account_at_checked(&id, &disabled, access_revision, check, fut).await
            })
        })
        .collect();
    let mut all = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(Some(mut snap)) = h.await {
            // Stamp each provider as it lands — not once after the
            // slowest sibling finishes — so fetchedAt is that card's
            // last success, not the batch join clock.
            if snap.status == "ok" && snap.fetched_at.is_none() {
                snap.fetched_at = Some(now_ms() as i64);
            }
            all.push(snap);
        }
    }
    if let Some(id) = missing_target {
        let name = full_policy.account_bindings.get(&id).map(|binding| binding.name.as_str()).unwrap_or(&id);
        let mut error = providers::Snapshot::error(&id, name,
            "Account identity could not be revalidated. Rediscover this account in Settings.".into());
        error.attempted_at = Some(now_ms() as i64);
        all.push(error);
    }
    let _publication = KEY_CARD_PUBLICATION.lock().unwrap_or_else(|e| e.into_inner());
    if !access_runtime().is_current(access_revision) { return Ok(empty_usage_result()); }
    retain_authorized_snapshots(&mut all, publication_policy);
    let current_key_card_generations = current_credential_scoped_generations(&all);
    let stale_key_card_ids = retain_current_key_card_results(
        &mut all,
        &expected_key_card_generations,
        &current_key_card_generations,
    );
    if !stale_key_card_ids.is_empty() {
        if !all
            .iter()
            .any(|snapshot| family_of(&snapshot.id) == "onenewapi")
        {
            enabled_ids.retain(|id| id != "onenewapi");
        }
        let mut failures = fail_state().lock().unwrap();
        for id in stale_key_card_ids {
            failures.remove(&id);
        }
    }
    let verify_commandcode = query_commandcode && (!automatic || provider_auto_refresh::enabled(&load_config(), "commandcode"));
    let commandcode_identity_now = if verify_commandcode {
        access_runtime().commit_if_current(access_revision, |current| {
            current.read_account("commandcode", providers::commandcode::default_identity).flatten()
        }).flatten()
    } else { None };
    let commandcode_swapped_mid_refresh = verify_commandcode && commandcode_identity_at_start != commandcode_identity_now;
    if commandcode_swapped_mid_refresh {
        fail_state().lock().unwrap_or_else(|e| e.into_inner()).remove("commandcode");
        for snapshot in all.iter_mut().filter(|snapshot| snapshot.id == "commandcode") {
            *snapshot = providers::Snapshot::error("commandcode", "CommandCode", "CommandCode key changed during refresh; refresh manually again".into());
            snapshot.attempted_at = Some(now_ms() as i64);
        }
    }
    let opencode_identity_now = read_usage_identity(&query_policy, automatic, "opencode", providers::opencode::default_identity).flatten();
    let opencode_swapped_mid_refresh = matches!(
        (&opencode_identity_at_start, &opencode_identity_now),
        (Some(old), Some(current)) if old != current
    );
    if opencode_swapped_mid_refresh {
        for s in &mut all {
            if s.id == "opencode" {
                *s = providers::Snapshot::error(
                    "opencode",
                    "OpenCode",
                    "OpenCode login changed during refresh.".into(),
                );
            }
        }
    }

    for s in &all {
        let log_family = family_of(&s.id);
        let log_id = if is_managed_key_card(&s.id) {
            log_family.as_str()
        } else {
            s.id.as_str()
        };
        eprintln!(
            "[pane] {}: {} ({} metrics){}",
            log_id,
            s.status,
            s.metrics.len(),
            s.error
                .as_deref()
                .map(|e| format!(" — {e}"))
                .unwrap_or_default()
        );
    }

    let mut staged_cache = last_ok().lock().unwrap_or_else(|e| e.into_inner()).clone();
    // A setting may close after batch admission but before the nested permit.
    // Use the actual composite outcome, not only the initial query policy.
    // Completed fresh wallet results are never replaced with older history.
    let saved_kimi = if target.as_deref() == Some("kimi") || !query_policy.allows_account("moonshot")
        || all.iter().any(|snap| snap.id == "kimi" && snap.wallet_refresh_skipped) {
        usage_publications().read(access_runtime())
            .and_then(|batch| batch.snapshots.into_iter().find(|snapshot| snapshot.id == "kimi"))
            .or_else(|| staged_cache.get("kimi").map(|cached| {
                let mut snap = cached.snap.clone();
                hydrate_fetch_time(&mut snap, cached.at);
                snap
            }))
    } else { None };
    let mut staged_stamp: Option<(PathBuf, Value)> = None;
    // Transient server errors (a 503, a timeout) shouldn't blank a card the
    // user was just reading: fall back to the last good snapshot, marked
    // stale so the UI can say "Outdated" with the real error on hover. The
    // cache survives app restarts. Manual-only Claude keeps saved history;
    // Sub2API keeps history until its credential context changes. Others keep the
    // existing one-day limit.
    {

        // Cache identity stamp (upstream's Phase 1): if a DIFFERENT account
        // signed into a default home since the cache was written, that
        // family's cached last-good snapshot belongs to the old account —
        // drop it instead of painting the wrong account's numbers under the
        // bare id. Extra-account cards are immune: their ids are derived
        // from the account identity itself.
        {
            let stamp_file = providers::config_dir().join("cache_identities.json");
            let current = json!({
                "commandcode": commandcode_identity_now,
                "claude": if query_claude { read_usage_identity(&query_policy, automatic, "claude", providers::claude::default_identity).flatten() } else { None },
                "codex": read_usage_identity(&query_policy, automatic, "codex", providers::codex::default_identity).flatten(),
                "opencode": read_usage_identity(&query_policy, automatic, "opencode", providers::opencode::default_identity).flatten(),
                "stepfun": read_usage_identity(&query_policy, automatic, "stepfun", providers::stepfun::default_identity).flatten(),
            });
            let stored: Value = std::fs::read_to_string(&stamp_file)
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_else(|| json!({}));
            let map = &mut staged_cache;
            let mut removed = false;
            let mut to_store = if target.is_some() {
                stored.as_object().cloned().unwrap_or_default()
            } else { serde_json::Map::new() };
            for fam in ["claude", "codex", "opencode", "stepfun", "commandcode"] {
                if !query_policy.allows_account(fam) || (automatic && !provider_auto_refresh::enabled(&load_config(), fam)) {
                    if let Some(old) = stored.get(fam) { to_store.insert(fam.into(), old.clone()); }
                    continue;
                }
                if target.as_ref().is_some_and(|id| id != fam) { continue; }
                let cur = current.get(fam).cloned().unwrap_or(Value::Null);
                let old = stored.get(fam).cloned().unwrap_or(Value::Null);
                // Only a KNOWN stored identity differing from a KNOWN
                // current one is evidence of an account swap. A missing
                // stamp (first launch after updating) or a momentarily
                // unreadable identity file must not dump the last-good
                // cache — that's the safety net, not a swap.
                if fam == "commandcode" && (old != cur || commandcode_swapped_mid_refresh) {
                    removed |= map.remove(fam).is_some();
                } else if !old.is_null() && !cur.is_null() && old != cur && map.remove(fam).is_some() {
                    removed = true;
                } else if fam == "opencode"
                    && opencode_swapped_mid_refresh
                    && map.remove(fam).is_some()
                {
                    // Mid-refresh A→B with two known fingerprints: drop
                    // the last-good so error restore cannot paint A as B.
                    removed = true;
                } else if fam == "opencode"
                    && old.is_null()
                    && !cur.is_null()
                    && map.remove(fam).is_some()
                {
                    // First stamp after upgrade: the cached snapshot
                    // predates identity tracking and may belong to a
                    // previous login. Drop it rather than pin it to the
                    // current key.
                    removed = true;
                }
                // And a transient null never OVERWRITES a known identity:
                // erasing it would make a swap that happens before the next
                // launch undetectable.
                to_store.insert(
                    fam.to_string(),
                    if cur.is_null() && !old.is_null() {
                        old
                    } else {
                        cur
                    },
                );
            }
            let _ = removed;
            let to_store = Value::Object(to_store);
            if to_store != stored { staged_stamp = Some((stamp_file, to_store)); }
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        {
            let map = &mut staged_cache;
            for s in all.iter_mut() {
                if is_credential_scoped_card(&s.id) {
                    let current = key_card_snapshot_generations([s.id.clone()]);
                    if expected_key_card_generations.get(&s.id) != current.get(&s.id) {
                        continue;
                    }
                }
                if s.id == "kimi" {
                    if let Some(previous) = &saved_kimi { preserve_unqueried_kimi_wallet(s, previous); }
                }
                // Plan bars can succeed while the folded Moonshot wallet
                // call fails; keep last-known API/Balance rows so Almost
                // Out and the tray pin don't blink off for one timeout.
                // Do not re-cache the patched snapshot — that would reset
                // `at` and keep serving the same balance forever.
                let mut skip_cache = false;
                if target.is_none() && s.id == "kimi" && s.status == "ok" && s.warning.is_some() {
                    if let Some(previous) = map.get("kimi") {
                        let age = now_ms - previous.at;
                        if age <= SNAPSHOT_CACHE_MS {
                            let n = s.metrics.len();
                            restore_kimi_wallet_rows(s, &previous.snap);
                            if s.metrics.len() > n {
                                let warning = s.warning.clone();
                                let wallet_attempted_at = s.wallet_history.as_ref().and_then(|history| history.attempted_at);
                                preserve_unqueried_kimi_wallet(s, &previous.snap);
                                if let Some(history) = s.wallet_history.as_mut() {
                                    history.warning = warning;
                                    history.attempted_at = wallet_attempted_at;
                                }
                                skip_cache = true;
                                s.attempt_failed = true;
                                if age > STALE_GRACE_MS {
                                    s.stale = true;
                                }
                            }
                        }
                    }
                }
                if s.status == "ok" && !skip_cache {
                    let at = s.fetched_at.unwrap_or(now_ms);
                    s.fetched_at = Some(at);
                    s.attempt_failed = false;
                    map.insert(
                        s.id.clone(),
                        CachedSnap {
                            at,
                            snap: s.clone(),
                        },
                    );

                } else if s.status == "error" {
                    if let Some(previous) = map.get(&s.id) {
                        let age = now_ms - previous.at;
                        let previous_at = previous.at;
                        if restore_last_success_after_error(s, &previous.snap, age) {
                            hydrate_fetch_time(s, previous_at);
                        }
                    }
                }
            }

        }
    }

    if let Some(previous) = &saved_kimi {
        for snapshot in all.iter_mut().filter(|s| s.id == "kimi") {
            preserve_unqueried_kimi_wallet(snapshot, previous);
        }
    }
    {
        // Append after the cache-write loop: replaying saved data is never a
        // successful fetch and must not refresh CachedSnap.at/fetched_at.
        append_manual_usage_snapshots(&mut all, &policy, &disabled, &staged_cache, &cfg);
    }

    // Recheck before publishing; the publication lock keeps local mutations
    // from interleaving cache updates, HTTP publication, and alerts.
    let current_key_card_generations = current_credential_scoped_generations(&all);
    let stale_key_card_ids = retain_current_key_card_results(
        &mut all,
        &expected_key_card_generations,
        &current_key_card_generations,
    );
    if !stale_key_card_ids.is_empty() {
        if !all
            .iter()
            .any(|snapshot| family_of(&snapshot.id) == "onenewapi")
        {
            enabled_ids.retain(|id| id != "onenewapi");
        }
        let mut failures = fail_state().lock().unwrap();
        for id in stale_key_card_ids {
            failures.remove(&id);
        }
    }

    // One Kimi card: Session / Weekly / API. Hide the leftover Moonshot
    // wallet card whenever the plan card is actually showing.
    if target.is_none() && (query_policy.allows_account("kimi") || !query_policy.allows_account("moonshot")) {
        fold_moonshot_into_kimi(&mut all);
    }

    // A user may disable a family or key while its request is in flight.
    let publish_cfg = config_with_defaults(load_config());
    let publish_disabled = publish_cfg.get("disabled").and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_default();
    all.retain(|snapshot| !card_is_disabled(&snapshot.id, &publish_disabled));
    // Local logs have no proven account attribution. Do not replay or
    // update account-capacity history from source-only statistics.
    retain_authorized_snapshots(&mut all, publication_policy);
    // Narrow queries must never prune other authorized cache entries. The full
    // backend policy remains the authority for retained publication and cache.
    let cache_policy = if target.is_some() { &full_policy } else { &policy };
    let Some((result, pending_alerts)) = usage_publications().commit_scoped(access_runtime(), access_revision, target.as_deref(), all, |authorized| {
        // No runtime re-entry or await inside this final synchronous fence.
        staged_cache.retain(|_, cached| cache_policy.allows_snapshot(&cached.snap.id, cached.snap.plan.as_deref()));
        for cached in staged_cache.values_mut() {
            usage_publication::filter_snapshot(&mut cached.snap, cache_policy);
        }
        let cache_persisted = persist_last_ok(&staged_cache).is_ok();
        *last_ok().lock().unwrap_or_else(|e| e.into_inner()) = staged_cache;
        if cache_persisted {
            if let Some((path, stamp)) = staged_stamp {
                let _ = std::fs::write(path, serde_json::to_string_pretty(&stamp).unwrap_or_default());
            }
        }
        let queried: Vec<_> = authorized.iter()
            .filter(|snapshot| target.as_ref().is_none_or(|id| &snapshot.id == id)).cloned().collect();
        alerts::evaluate(&queried, &cfg)
    }) else { return Ok(empty_usage_result()); };
    for alert in pending_alerts {
        if !access_runtime().is_current(access_revision) { return Ok(empty_usage_result()); }
        use tauri_plugin_notification::NotificationExt;
        let _ = app.notification().builder().title(&alert.title).body(&alert.body).show();
    }
    Ok(if access_runtime().is_current(access_revision) { result } else { empty_usage_result() })
}

/// The previous run's last-good snapshots, straight from the disk cache —
/// the instant first paint at launch. Cards show numbers in milliseconds
/// instead of a blank "Refreshing…" while the slowest provider answers
/// (at boot, with the network still coming up, that wait ran 30-40 s).
/// Auto-disabled providers remain explicitly historical until a manual refresh.
/// Reading their app-owned cache never opens credentials or identities.
#[tauri::command]
fn cached_usage() -> usage_publication::UsageResult {
    let _publication = KEY_CARD_PUBLICATION.lock().unwrap_or_else(|e| e.into_inner());
    // A late startup-cache command must not replace a live success/error with
    // disk history. Cache staging and live commits share this publication lock.
    if let Some(current) = usage_publications().read(access_runtime()) { return current; }
    let (mut policy, access_revision) = access_runtime().snapshot();
    if policy.enabled_accounts.is_empty() { return empty_usage_result(); }
    const MAX_STALE_MS: i64 = SNAPSHOT_CACHE_MS;
    let Ok(raw) = std::fs::read_to_string(providers::config_dir().join("last_snapshots.json"))
    else {
        return empty_usage_result();
    };
    let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, CachedSnap>>(&raw)
    else {
        return empty_usage_result();
    };

    let cfg = config_with_defaults(load_config());
    let disabled: Vec<String> = cfg
        .get("disabled")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();



    policy.enabled_accounts.retain(|id| !disabled.contains(id));

    // Same account-swap rule as the live path: if a different account
    // signed into a default home since the cache was written, that
    // family's bare-id entry belongs to the old account — never paint it.
    // Auto-disabled families paint explicitly historical account details
    // without identity reads; their next manual query performs the check.
    let stored: Value =
        std::fs::read_to_string(providers::config_dir().join("cache_identities.json"))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| json!({}));
    let mut query_policy = policy.clone();
    query_policy.enabled_accounts.retain(|id| provider_auto_refresh::enabled(&cfg, id));
    let swapped: Vec<&str> = [
        ("codex", read_usage_identity(&query_policy, true, "codex", providers::codex::default_identity).flatten()),
        ("opencode", read_usage_identity(&query_policy, true, "opencode", providers::opencode::default_identity).flatten()),
        ("stepfun", read_usage_identity(&query_policy, true, "stepfun", providers::stepfun::default_identity).flatten()),
    ]
    .into_iter()
    .filter(|(fam, current)| {
        let old = stored.get(fam).cloned().unwrap_or(Value::Null);
        matches!((current, &old), (Some(cur), Value::String(o)) if cur != o)
    })
    .map(|(fam, _)| fam)
    .collect();

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut out: Vec<providers::Snapshot> = map
        .iter()
        .filter(|(id, c)| {
            (provider_auto_refresh::enabled(&cfg, id) && !matches!(family_of(id).as_str(), "sub2api" | "onenewapi") && now_ms - c.at <= MAX_STALE_MS)
                && policy.allows_account(id)
                && !card_is_disabled(id, &disabled)
                && !swapped.iter().any(|f| f == id)
        })
        .map(|(_, c)| {
            let mut s = c.snap.clone();
            s.stale = true;
            hydrate_fetch_time(&mut s, c.at);
            s
        })
        .collect();
    append_manual_usage_snapshots(&mut out, &policy, &disabled, &map, &cfg);
    retain_authorized_snapshots(&mut out, &policy);
    fold_moonshot_into_kimi(&mut out);
    out.sort_by(|a, b| a.id.cmp(&b.id));
    usage_publications().publish(access_runtime(), access_revision, out).unwrap_or_else(empty_usage_result)
}

/// Computes local spend (Today / Yesterday / Last 30 Days) from the CLIs'
/// own session logs. Heavy file IO, so it runs on a blocking thread.
#[tauri::command]
async fn fetch_spend(reason: Option<UsageRefreshReason>) -> SpendResult {
    let automatic = matches!(reason.unwrap_or_default(), UsageRefreshReason::Automatic);
    let scan_policy = local_scan_policy();
    let cursor_access_revision = scan_policy.revision;
    eprintln!("[pane] spend: scan starting");
    let started = std::time::Instant::now();
    // Cursor's CSV export needs the async client; fetch it here and hand it
    // to the blocking scan. Unlike every other spend source it's an
    // authenticated NETWORK call, so it honors the disabled toggle the same
    // way fetch_usage does — a switched-off Cursor makes no requests.
    let cursor_disabled = config_with_defaults(load_config())
        .get("disabled")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some("cursor")));
    // CSV is a network call. Don't make the local log walk sit behind it —
    // Codex/Claude appends are the slow part, and they don't need Cursor.
    let csv_task = async {
        if cursor_disabled || (automatic && !provider_auto_refresh::enabled(&load_config(), "cursor")) {
            None
        } else {
            let check: Option<access_policy::OperationCheck> = automatic.then(|| {
                Arc::new(|account: &str| provider_auto_refresh::enabled(&load_config(), account)) as access_policy::OperationCheck
            });
            access_runtime().run_account_at_checked("cursor", &[], cursor_access_revision, check,
                providers::cursor::fetch_usage_csv()).await.flatten()
        }
    };
    let scan_task = tauri::async_runtime::spawn_blocking(move || spend::collect(&scan_policy, None));
    let (cursor_csv, local) = tokio::join!(csv_task, scan_task);
    let mut result = local.unwrap_or_default();
    let cursor_csv = cursor_csv.filter(|_| access_runtime().is_current(cursor_access_revision)
        && access_runtime().snapshot().0.allows_account("cursor"));
    let cursor_refreshed = cursor_csv.is_some();
    if let Some(csv) = cursor_csv {
        let cursor = spend::cursor_from_csv(&csv);
        if spend::provider_spend_has_data(&cursor) {
            result.push(cursor);
        }
    }
    eprintln!(
        "[pane] spend: {} providers in {:?}",
        result.len(),
        started.elapsed()
    );
    // This is only a replay hint. The frontend retains existing authorized CSV
    // rows and their clocks; local scans do not become a new Cursor observation.
    let preserve_cursor = automatic && !cursor_disabled && !cursor_refreshed
        && !provider_auto_refresh::enabled(&load_config(), "cursor");
    access_runtime().commit_if_current(cursor_access_revision, |policy| SpendResult {
        revision: cursor_access_revision, rows: result,
        preserve_cursor: preserve_cursor && policy.allows_account("cursor"),
    }).unwrap_or_else(|| SpendResult { revision: access_runtime().snapshot().1, rows: Vec::new(), preserve_cursor: false })
}

#[tauri::command]
fn pricing_status() -> pricing::PricingSyncResult { pricing::status() }

#[derive(serde::Serialize)]
struct PricingUpdate { pricing: pricing::PricingSyncResult, spend: SpendResult }

/// Downloads only public catalogs, then recomputes authorized local estimates.
/// Cursor's existing authoritative export is preserved by the frontend.
#[tauri::command]
async fn sync_prices() -> Result<PricingUpdate, String> {
    let pricing = pricing::sync_catalogs().await?;
    let policy = local_scan_policy();
    let revision = policy.revision;
    let rows = tauri::async_runtime::spawn_blocking(move || spend::collect(&policy, None))
        .await.map_err(|_| "Prices saved; local recomputation could not finish".to_string())?;
    let spend = access_runtime().commit_if_current(revision, |_| SpendResult { revision, rows, preserve_cursor: false })
        .unwrap_or_else(|| SpendResult { revision: access_runtime().snapshot().1, rows: Vec::new(), preserve_cursor: false });
    Ok(PricingUpdate { pricing, spend })
}

/// The key a provider's credential file currently holds (None when the
/// file is absent or unreadable). Used to tell a real credential change
/// from a re-save of the identical key, so an unchanged save doesn't dump
/// the cached snapshot. The key never leaves this comparison — not
/// logged, not returned, not published.
fn stored_pane_api_key(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let doc = serde_json::from_str::<Value>(&raw).ok()?;
    doc.get("apiKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

struct CreditMeterBindGuard {
    ids: Vec<String>,
}

impl CreditMeterBindGuard {
    fn begin(ids: Vec<String>) -> Self {
        for id in &ids {
            providers::bind_credit_meter_generation(
                id,
                providers::credit_baseline_generation(id),
            );
        }
        Self { ids }
    }
}

impl Drop for CreditMeterBindGuard {
    fn drop(&mut self) {
        for id in &self.ids {
            providers::unbind_credit_meter_generation(id);
        }
    }
}

fn credit_meter_bind_ids(id: &str) -> Vec<String> {
    match id {
        "kimi" => vec!["kimi".into(), "moonshot".into()],
        _ => vec![id.to_string()],
    }
}

fn api_key_context_is_dirty(dir: &Path, provider: &str) -> bool {
    let snapshot_ids = api_key_snapshot_ids(provider);
    let baseline_ids = api_key_baseline_ids(provider);
    if SNAPSHOT_CACHE_NEEDS_FLUSH.load(Ordering::Acquire) {
        return true;
    }
    {
        let cache = last_ok().lock().unwrap();
        if snapshot_ids.iter().any(|id| cache.contains_key(id)) {
            return true;
        }
    }
    {
        let failures = fail_state().lock().unwrap();
        if snapshot_ids.iter().any(|id| failures.contains_key(id)) {
            return true;
        }
    }
    providers::credit_baselines_contain(dir, &baseline_ids)
}

fn context_cleanup_error(error: String) -> String {
    format!("the key change is saved, but clearing the previous key's cached data failed: {error}")
}

/// A credential actually changed: everything the old key produced — the
/// last-good snapshot (memory + disk), the local HTTP publication, the
/// fail-state cooldown, the alert history, and the credit high-water
/// mark — belongs to the old account. Moonshot rotation also drops the
/// folded Kimi snapshot; each key's credit baseline is forgotten alone.
fn invalidate_api_key_context(dir: &Path, provider: &str) -> Result<(), String> {
    let snapshot_ids = api_key_snapshot_ids(provider);
    let baseline_ids = api_key_baseline_ids(provider);
    let _mutation = KeyCardMutationGuard::begin(snapshot_ids.clone());
    let snap_err = forget_provider_snapshots_inner(&snapshot_ids, true)
        .err()
        .map(context_cleanup_error);
    let base_err = providers::forget_credit_baselines_in(dir, &baseline_ids)
        .err()
        .map(context_cleanup_error);
    // A MiniMax key change also forgets the remembered mcode plan tier —
    // a pasted key must never inherit another account's tier.
    let tier_err = if provider == "minimax" {
        providers::minimax::forget_remembered_tier_in(dir)
            .err()
            .map(context_cleanup_error)
    } else {
        None
    };
    [snap_err, base_err, tier_err]
        .into_iter()
        .flatten()
        .reduce(|left, right| format!("{left}; {right}"))
        .map_or(Ok(()), Err)
}

fn set_api_key_in(dir: &Path, provider: &str, key: &str) -> Result<(), String> {
    if !is_plain_api_key_provider(provider) {
        return Err(format!("unknown provider: {provider}"));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("create config dir: {e}"))?;
    if provider == "commandcode" {
        let ids = vec![provider.to_string()];
        let _mutation = KeyCardMutationGuard::begin(ids.clone());
        return providers::commandcode::save_key_in(dir, key.trim(), || {
            forget_provider_snapshots_inner(&ids, true)
        });
    }
    let path = dir.join(format!("{provider}.json"));
    let previous_key = stored_pane_api_key(&path);
    let key = key.trim();
    if key.is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("remove key file: {e}")),
        }
        if previous_key.is_some()
            || api_key_context_is_dirty(dir, provider)
            || (provider == "minimax" && providers::minimax::remembered_tier_exists_in(dir))
        {
            invalidate_api_key_context(dir, provider)?;
        }
        return Ok(());
    }
    // A MiniMax key change stashes the remembered mcode tier BEFORE the
    // new key lands: if the stash fails the old key stays in place and a
    // retry still sees a differing key; if the key write then fails the
    // stash goes back so the old key keeps its tier. (The invalidation
    // below retries the same delete — NotFound is Ok — so a succeeded
    // change is idempotent.)
    let stashed = provider == "minimax"
        && previous_key.as_deref() != Some(key)
        && providers::minimax::stash_remembered_tier_in(dir).map_err(context_cleanup_error)?;
    let raw = serde_json::json!({ "apiKey": key }).to_string();
    if let Err(e) = private_file::atomic_write(&path, &raw) {
        if stashed {
            providers::minimax::restore_stashed_tier_in(dir);
        }
        return Err(format!("write key file: {e}"));
    }
    if stashed {
        providers::minimax::discard_stashed_tier_in(dir);
    }
    // Same-key retries still invalidate when a previous cleanup left
    // snapshots, cooldowns, or credit baselines behind.
    if previous_key.as_deref() != Some(key) || api_key_context_is_dirty(dir, provider) {
        invalidate_api_key_context(dir, provider)?;
    }
    Ok(())
}

/// Saves (or clears, when `key` is empty) a user-pasted API key to
/// %APPDATA%\Pane\<provider>.json.
#[tauri::command]
fn set_api_key(provider: String, key: String) -> Result<(), String> {
    set_api_key_in(&providers::config_dir(), &provider, &key)?;
    if provider == "qwen" { providers::qwen::reset_quota_cooldown(); }
    Ok(())
}

































/// Opens a provider quick link in the default browser. Only plain web URLs —
/// nothing that could launch a program.
#[tauri::command]
fn open_link(app: tauri::AppHandle, url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http(s) links allowed".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("open link: {e}"))
}

/// A share card is a few hundred KB of PNG at 2x scale; 8 MB of base64
/// (6 MB decoded) leaves generous headroom while bounding what any code
/// running in the WebView can hand us.
const MAX_SHARE_PNG_BASE64: usize = 8 * 1024 * 1024;
/// Raw RGBA is 4 bytes per pixel, so 16 M pixels caps the expansion at
/// 64 MB. Real cards are ~1200x2400 (≈3 M pixels).
const MAX_SHARE_PNG_PIXELS: u64 = 16_000_000;

/// Reads width/height out of a PNG's IHDR chunk, which is always the first
/// chunk right after the 8-byte signature. Checking the declared dimensions
/// *before* handing the bytes to a decoder is what keeps a decompression
/// bomb (tiny file, billions of pixels) from being expanded at all.
fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIG || &bytes[12..16] != b"IHDR" {
        return Err("not a PNG".into());
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if w == 0 || h == 0 {
        return Err("empty image".into());
    }
    Ok((w, h))
}

/// Puts a share-card PNG (rendered by the frontend on a canvas) onto the
/// Windows clipboard as a real image.
///
/// Every command is callable by whatever JavaScript runs in the WebView, so
/// the encoded size and the declared pixel count are both bounded before any
/// decoding happens — otherwise a crafted PNG could force a multi-gigabyte
/// RGBA allocation and take the tray process down.
#[tauri::command]
fn copy_share_image(png_base64: String) -> Result<(), String> {
    use base64::Engine;
    let png_base64 = png_base64.trim();
    if png_base64.len() > MAX_SHARE_PNG_BASE64 {
        return Err("share image too large".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64)
        .map_err(|e| format!("decode png: {e}"))?;
    let (dw, dh) = png_dimensions(&bytes)?;
    if u64::from(dw) * u64::from(dh) > MAX_SHARE_PNG_PIXELS {
        return Err("share image too large".into());
    }
    let img = tauri::image::Image::from_bytes(&bytes).map_err(|e| format!("parse png: {e}"))?;
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w != dw as usize || h != dh as usize {
        return Err("share image dimensions mismatch".into());
    }
    let rgba = img.rgba().to_vec();
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("clipboard: {e}"))?;
    clipboard
        .set_image(arboard::ImageData {
            width: w,
            height: h,
            bytes: rgba.into(),
        })
        .map_err(|e| format!("copy image: {e}"))
}

/// (Re-)registers the global toggle-popover shortcut. An empty string clears
/// it. The accelerator uses Tauri syntax, e.g. "Ctrl+Shift+U".
fn register_shortcut(app: &tauri::AppHandle, accel: &str) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let accel = accel.trim();
    if accel.is_empty() {
        return Ok(());
    }
    let shortcut: Shortcut = accel
        .parse()
        .map_err(|_| format!("could not parse shortcut \"{accel}\""))?;
    gs.on_shortcut(shortcut, |app, _shortcut, event| {
        if event.state() == ShortcutState::Pressed {
            let pos = app
                .cursor_position()
                .unwrap_or(tauri::PhysicalPosition::new(1200.0, 700.0));
            toggle_popover(app, pos);
        }
    })
    .map_err(|e| format!("register shortcut: {e}"))
}

#[tauri::command]
fn set_shortcut(app: tauri::AppHandle, shortcut: String) -> Result<(), String> {
    register_shortcut(&app, &shortcut)
}

fn current_disabled_accounts() -> Vec<String> {
    load_config().get("disabled").and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Spends one banked Codex rate-limit reset credit. Irreversible — the
/// frontend shows a confirm dialog before calling this.
#[tauri::command]
async fn codex_redeem_credit(
    credit_id: String,
    provider_id: Option<String>,
    redeem_request_id: Option<String>,
) -> Result<providers::codex::RedeemOutcome, String> {
    // provider_id routes multi-account redeems; absent = the default card
    // (older frontend builds during an update overlap). redeem_request_id
    // is the frontend's per-credit idempotency key.
    let pid = provider_id.unwrap_or_else(|| "codex".into());
    if pid.split('@').next() != Some("codex") { return Err("Invalid Codex account".into()); }
    access_runtime().run_account(&pid, &current_disabled_accounts(), providers::codex::redeem_credit(&pid, &credit_id, redeem_request_id)).await
        .ok_or_else(|| "Account is disabled or authorization changed; check the provider before retrying a redemption".to_string())?
}

/// Spends one banked Claude limit reset (cedar_ember grant). Irreversible
/// — the frontend shows the same confirm dialog as Codex's before
/// calling this.
#[tauri::command]
async fn claude_redeem_credit(
    credit_id: String,
    provider_id: Option<String>,
    redeem_request_id: Option<String>,
) -> Result<providers::codex::RedeemOutcome, String> {
    let pid = provider_id.unwrap_or_else(|| "claude".into());
    if pid.split('@').next() != Some("claude") { return Err("Invalid Claude account".into()); }
    access_runtime().run_account(&pid, &current_disabled_accounts(), providers::claude::redeem_credit(&pid, &credit_id, redeem_request_id.as_deref())).await
        .ok_or_else(|| "Account is disabled or authorization changed; check the provider before retrying a redemption".to_string())?
}

















// ---------------------------------------------------------------------------
// Tray + popover window plumbing
// ---------------------------------------------------------------------------

// Clicking the tray icon while the popover is open first steals focus
// (which hides the window) and then delivers the click event. Without a
// guard, that click would instantly re-open the window the user just
// closed. We remember when the last auto-hide happened and ignore tray
// clicks that arrive right after it.
static LAST_AUTO_HIDE_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Tells WebView2 to release memory while the popover is hidden and return
/// to normal when it shows. Tauri doesn't expose wry's setter for this, so
/// we make the same COM calls wry does (SetMemoryUsageTargetLevel).
fn set_webview_memory_level(window: &tauri::WebviewWindow, low: bool) {
    let _ = window.with_webview(move |webview| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL,
        };
        use windows_core::Interface;
        if let Ok(core) = webview.controller().CoreWebView2() {
            if let Ok(wv19) = core.cast::<ICoreWebView2_19>() {
                let level = COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL(if low { 1 } else { 0 });
                let _ = wv19.SetMemoryUsageTargetLevel(level);
            }
        }
    });
}

/// Hide the dashboard without the tray-click reopen dance. Esc on the
/// dashboard (and the hide half of the tray toggle) both land here so
/// the webview still drops to the low-memory target.
#[tauri::command]
fn hide_popover(app: tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        set_webview_memory_level(&window, true);
    }
}

#[derive(Clone, Copy)]
struct ScreenRect {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

#[derive(Clone, Copy, Default)]
struct WindowInsets {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

fn popover_origin(
    click: (i64, i64),
    size: (i64, i64),
    monitor: ScreenRect,
    work: ScreenRect,
) -> (i64, i64) {
    enum TaskbarEdge {
        Bottom,
        Top,
        Left,
        Right,
    }

    let left = (work.x - monitor.x).max(0);
    let top = (work.y - monitor.y).max(0);
    let right = (monitor.x + monitor.width - work.x - work.width).max(0);
    let bottom = (monitor.y + monitor.height - work.y - work.height).max(0);
    let max_inset = bottom.max(top).max(left).max(right);
    let edge = [
        (TaskbarEdge::Bottom, bottom),
        (TaskbarEdge::Top, top),
        (TaskbarEdge::Left, left),
        (TaskbarEdge::Right, right),
    ]
        .into_iter()
        .find_map(|(edge, inset)| (inset == max_inset).then_some(edge))
        .unwrap_or(TaskbarEdge::Bottom);

    let (x, y) = match edge {
        TaskbarEdge::Top => (click.0 - size.0, work.y),
        TaskbarEdge::Left => (work.x, click.1 - size.1),
        TaskbarEdge::Right => (work.x + work.width - size.0, click.1 - size.1),
        TaskbarEdge::Bottom => (click.0 - size.0, work.y + work.height - size.1),
    };
    let clamp = |origin: i64, start: i64, extent: i64, length: i64| {
        if length >= extent {
            start
        } else {
            origin.clamp(start, start + extent - length)
        }
    };
    (
        clamp(x, work.x, work.width, size.0),
        clamp(y, work.y, work.height, size.1),
    )
}

fn popover_outer_origin(
    click: (i64, i64),
    outer_size: (i64, i64),
    monitor: ScreenRect,
    work: ScreenRect,
    insets: WindowInsets,
) -> (i64, i64) {
    let inner_size = (
        (outer_size.0 - insets.left - insets.right).max(0),
        (outer_size.1 - insets.top - insets.bottom).max(0),
    );
    let (x, y) = popover_origin(click, inner_size, monitor, work);
    (x - insets.left, y - insets.top)
}

fn window_insets(window: &tauri::WebviewWindow, outer_size: tauri::PhysicalSize<u32>) -> WindowInsets {
    let (Ok(outer), Ok(inner), Ok(inner_size)) = (
        window.outer_position(),
        window.inner_position(),
        window.inner_size(),
    ) else {
        return WindowInsets::default();
    };
    let valid = |value: i64| if (0..=128).contains(&value) { value } else { 0 };
    let left = valid(i64::from(inner.x) - i64::from(outer.x));
    let top = valid(i64::from(inner.y) - i64::from(outer.y));
    WindowInsets {
        left,
        top,
        right: valid(i64::from(outer_size.width) - i64::from(inner_size.width) - left),
        bottom: valid(i64::from(outer_size.height) - i64::from(inner_size.height) - top),
    }
}

fn toggle_popover(app: &tauri::AppHandle, click: tauri::PhysicalPosition<f64>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        set_webview_memory_level(&window, true);
        return;
    }

    if now_ms().saturating_sub(LAST_AUTO_HIDE_MS.load(Ordering::Relaxed)) < 300 {
        return;
    }

    set_webview_memory_level(&window, false);

    let size = window
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(380, 600));
    // The widget reopens wherever the user last dragged it.
    let anchor = !widget::widget_mode();
    if let Some(monitor) = window
        .monitor_from_point(click.x, click.y)
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
        .filter(|_| anchor)
    {
        let rect = ScreenRect {
            x: i64::from(monitor.position().x),
            y: i64::from(monitor.position().y),
            width: i64::from(monitor.size().width),
            height: i64::from(monitor.size().height),
        };
        let area = monitor.work_area();
        let work = ScreenRect {
            x: i64::from(area.position.x),
            y: i64::from(area.position.y),
            width: i64::from(area.size.width),
            height: i64::from(area.size.height),
        };
        let (x, y) = popover_outer_origin(
            (click.x as i64, click.y as i64),
            (i64::from(size.width), i64::from(size.height)),
            rect,
            work,
            window_insets(&window, size),
        );
        let _ = window.set_position(tauri::PhysicalPosition::new(
            x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        ));
    }
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.emit("popover-shown", ());
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Second launches just poke the existing instance's popover open
        // instead of spawning a duplicate tray icon (Mac parity).
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let pos = app
                .cursor_position()
                .unwrap_or(tauri::PhysicalPosition::new(1200.0, 700.0));
            toggle_popover(app, pos);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())


        .invoke_handler(tauri::generate_handler![
            fetch_usage,
            cached_usage,
            fetch_spend,
            pricing_status,
            sync_prices,
            set_api_key,
            get_config,
            set_config,
            set_provider_family,
            set_provider_account,
            get_provider_modes,
            set_provider_region,
            discover_provider_account,
            reset_provider_access,
            set_scan_source,
            get_scan_sources,
            configure_scan_source,
            check_dir,
            system_ui_locale,
            get_autostart,
            set_autostart,
            sync_tray_surfaces,
            open_link,
            copy_share_image,
            set_shortcut,
            codex_redeem_credit,
            claude_redeem_credit,
            widget::widget_apply,
            widget::widget_start_drag,
            hide_popover
        ])
        .setup(|app| {
            if let Err(error) = spend::migrate_legacy_cache() { eprintln!("[pane] {error}"); }

            let quit = MenuItem::with_id(
                app,
                "quit",
                i18n::quit_label(&config_with_defaults(load_config())),
                true,
                None::<&str>,
            )?;
            let menu = Menu::with_items(app, &[&quit])?;

            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Pane")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id.as_ref() == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        position,
                        ..
                    } = event
                    {
                        toggle_popover(tray.app_handle(), position);
                    }
                })
                .build(app)?;

            // The popover starts hidden, so start the webview in low-memory
            // mode too; it flips to normal the first time it is shown.
            if let Some(wv) = app.get_webview_window("main") {
                set_webview_memory_level(&wv, true);
            }

            widget::init_from_config(&load_config(), app.handle());
            widget::spawn_taskbar_keeper(app.handle());

            httpapi::start();

            let saved_shortcut = load_config()
                .get("shortcut")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Err(e) = register_shortcut(app.handle(), &saved_shortcut) {
                eprintln!("[pane] shortcut: {e}");
            }

            // Start with Windows is on by default (like the Mac app's
            // launch-at-login) and re-asserted each launch so the registry
            // entry follows the exe if it moves — e.g. loose exe → installed.
            // Only an explicit "off" in Settings is respected. Skipped in dev
            // builds so the debug exe never registers itself.
            if !cfg!(debug_assertions) {
                let wants_autostart = load_config()
                    .get("autostart")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                if wants_autostart {
                    use tauri_plugin_autostart::ManagerExt;
                    let _ = app.autolaunch().enable();
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::Focused(false) = event {
                    // The widget stays on screen instead of auto-hiding.
                    if widget::widget_mode() {
                        return;
                    }
                    if window.hide().is_ok() {
                        LAST_AUTO_HIDE_MS.store(now_ms(), Ordering::Relaxed);
                        if let Some(wv) = window.app_handle().get_webview_window("main") {
                            set_webview_memory_level(&wv, true);
                        }
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{
        current_credential_scoped_generations, guarded, is_credential_scoped_card,
        is_plain_api_key_provider, set_api_key_in, stored_pane_api_key,
        cached_kimi_ok_from, card_is_disabled,
        commit_strip_state_after_apply, fail_state, load_config_from, set_config_in,
        fold_moonshot_into_kimi, forget_provider_snapshot,
        is_kimi_wallet_label, last_ok, key_card_snapshot_generations, persist_last_ok_at,
        persist_config_at, ConfigPersistIo,
        key_cards_purge_restore_patch, purge_key_cards_from_config,
        rename_cached_snapshot, rename_cached_snapshot_in, rename_cached_snapshots_in,
        restore_kimi_wallet_rows,
        hydrate_fetch_time, restore_last_success_after_error,
        retain_current_key_card_results, strip_entry_application_order, strip_icon_ids_to_clear,
        strip_is_active, strip_reset_ids, CachedSnap, FailState,
        KeyCardMutationGuard, StripEntry, SNAPSHOT_CACHE_MS, SNAPSHOT_CACHE_NEEDS_FLUSH,
        STALE_GRACE_MS, TEST_PERSIST_LAST_OK_FAIL,
    };
    use crate::alerts;
    use crate::providers::{Metric, Snapshot};
    use serde_json::{json, Value};
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::Ordering;

    fn strip_entry(id: &str, value: u32) -> StripEntry {
        StripEntry {
            id: id.into(),
            logo: vec![0; 32 * 32 * 4],
            labels: vec!["Session".into()],
            values: vec![value],
            tooltip: id.into(),
        }
    }



    #[test]
    fn kimi_wallet_labels() {
        assert!(is_kimi_wallet_label("API"));
        assert!(is_kimi_wallet_label("Balance"));
        assert!(!is_kimi_wallet_label("Session"));
        assert!(!is_kimi_wallet_label("Weekly"));
    }

    #[test]
    fn restore_wallet_rows_when_api_missing() {
        let mut current = Snapshot::ok(
            "kimi",
            "Kimi Code",
            None,
            vec![Metric::progress("Session", 0.0, None)],
        );
        current.warning = Some("Moonshot API wallet couldn't refresh".into());
        let previous = Snapshot::ok(
            "kimi",
            "Kimi Code",
            None,
            vec![
                Metric::progress("Session", 10.0, None),
                Metric::progress("API", 24.0, None),
                Metric::text("Balance", "$152.00".into()),
            ],
        );
        restore_kimi_wallet_rows(&mut current, &previous);
        let labels: Vec<_> = current.metrics.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["Session", "API", "Balance"]);
    }

    #[test]
    fn restore_wallet_rows_skips_when_api_present() {
        let mut current = Snapshot::ok(
            "kimi",
            "Kimi Code",
            None,
            vec![
                Metric::progress("Session", 0.0, None),
                Metric::progress("API", 1.0, None),
            ],
        );
        let previous = Snapshot::ok(
            "kimi",
            "Kimi Code",
            None,
            vec![Metric::progress("API", 99.0, None)],
        );
        restore_kimi_wallet_rows(&mut current, &previous);
        let api = current.metrics.iter().find(|m| m.label == "API").unwrap();
        assert!((api.used_percent.unwrap() - 1.0).abs() < 0.01);
    }

    #[test]
    fn cached_kimi_ok_ignores_stale_or_missing_entries() {
        let now = 1_800_000_000_000i64;
        let fresh = json!({"kimi": {"at": now - 60_000, "snap": {"status": "ok"}}});
        assert!(cached_kimi_ok_from(&fresh, now));
        let old = json!({"kimi": {"at": now - SNAPSHOT_CACHE_MS - 1, "snap": {"status": "ok"}}});
        assert!(!cached_kimi_ok_from(&old, now));
        let err = json!({"kimi": {"at": now, "snap": {"status": "error"}}});
        assert!(!cached_kimi_ok_from(&err, now));
        assert!(!cached_kimi_ok_from(&json!({}), now));
    }

    #[test]
    fn tray_strip_order_change_rebuilds_all_pairs_right_to_left() {
        let previous = vec![strip_entry("claude", 50), strip_entry("codex", 60)];
        let next = vec![strip_entry("codex", 60), strip_entry("claude", 50)];

        let reset_ids = strip_reset_ids(&previous, &next);
        let application_ids: Vec<&str> = strip_entry_application_order(&next, true)
            .into_iter()
            .map(|entry| entry.id.as_str())
            .collect();

        assert_eq!(reset_ids, vec!["claude", "codex"]);
        assert_eq!(application_ids, vec!["claude", "codex"]);
    }

    #[test]
    fn tray_strip_value_change_keeps_existing_pairs() {
        let previous = vec![strip_entry("claude", 50), strip_entry("codex", 60)];
        let next = vec![strip_entry("claude", 40), strip_entry("codex", 30)];

        assert!(strip_reset_ids(&previous, &next).is_empty());
    }

    #[test]
    fn failed_tray_strip_clear_invalidates_cache_so_retry_rebuilds() {
        let previous = vec![strip_entry("claude", 50), strip_entry("codex", 60)];
        let same_order = previous.clone();
        let reordered = vec![strip_entry("codex", 60), strip_entry("claude", 50)];
        let mut cached = previous.clone();

        let result: Result<(), String> = Err("native tray update failed".into());
        assert!(commit_strip_state_after_apply(&mut cached, &same_order, result).is_err());
        cached.clear();

        assert!(!strip_reset_ids(&cached, &same_order).is_empty());
        assert!(!strip_reset_ids(&cached, &reordered).is_empty());
    }

    #[test]
    fn successful_tray_strip_apply_commits_the_new_state() {
        let previous = vec![strip_entry("claude", 50), strip_entry("codex", 60)];
        let next = vec![strip_entry("codex", 60), strip_entry("claude", 50)];
        let mut cached = previous;

        assert!(commit_strip_state_after_apply(&mut cached, &next, Ok(())).is_ok());
        assert!(strip_reset_ids(&cached, &next).is_empty());
    }

    #[test]
    fn strip_is_inactive_when_apply_failed() {
        assert!(!strip_is_active(false, &[strip_entry("claude", 50)]));
    }

    #[test]
    fn strip_is_inactive_when_entries_are_empty() {
        assert!(!strip_is_active(true, &[]));
        assert!(!strip_is_active(false, &[]));
    }

    #[test]
    fn strip_is_active_when_apply_succeeded_with_entries() {
        assert!(strip_is_active(true, &[strip_entry("claude", 50)]));
    }

    #[test]
    fn strip_clear_ids_include_family_and_account_cards() {
        let known = vec![strip_entry("claude@work", 50)];
        let attempted = vec![strip_entry("codex", 40)];
        let ids = strip_icon_ids_to_clear(&known, &attempted);
        assert!(ids.contains(&"claude".into()));
        assert!(ids.contains(&"claude@work".into()));
        assert!(ids.contains(&"codex".into()));
    }













    #[test]
    fn recent_error_fallback_within_grace_is_not_marked_stale() {
        let previous = Snapshot::ok(
            "codex",
            "Codex",
            None,
            vec![Metric::progress("Weekly", 25.0, None)],
        );
        let mut current = Snapshot::error("codex", "Codex", "timeout".into());

        assert!(restore_last_success_after_error(
            &mut current,
            &previous,
            1_000
        ));
        assert_eq!(current.status, "ok");
        assert!(!current.stale);
        assert!(current.attempt_failed);
        assert_eq!(current.warning, None);
        assert_eq!(current.metrics[0].used_percent, Some(25.0));
    }

    #[test]
    fn recent_error_fallback_after_grace_is_marked_stale() {
        let previous = Snapshot::ok(
            "codex",
            "Codex",
            None,
            vec![Metric::progress("Weekly", 25.0, None)],
        );
        let mut current = Snapshot::error("codex", "Codex", "timeout".into());

        assert!(restore_last_success_after_error(
            &mut current,
            &previous,
            STALE_GRACE_MS + 1,
        ));
        assert_eq!(current.status, "ok");
        assert!(current.stale);
        assert!(current.attempt_failed);
        assert_eq!(current.warning.as_deref(), Some("timeout"));
        assert_eq!(current.metrics[0].used_percent, Some(25.0));
    }

    #[test]
    fn old_cache_without_fetched_at_publishes_the_cache_clock() {
        let raw = r#"{"codex":{"at":1800000000000,"snap":{"id":"codex","name":"Codex","plan":null,"status":"ok","error":null,"metrics":[],"stale":false,"warning":null}}}"#;
        let map: std::collections::HashMap<String, CachedSnap> =
            serde_json::from_str(raw).unwrap();
        let entry = &map["codex"];
        assert!(entry.snap.fetched_at.is_none());
        let mut s = entry.snap.clone();
        hydrate_fetch_time(&mut s, entry.at);
        let json = crate::httpapi::provider_json(&s, "2026-09-05T00:00:00Z");
        assert_eq!(json["fetchedAt"], "2027-01-15T08:00:00Z");
        assert_eq!(json["status"], "ok");
    }

    #[test]
    fn expired_error_fallback_is_not_restored() {
        let previous = Snapshot::ok(
            "codex",
            "Codex",
            None,
            vec![Metric::progress("Weekly", 25.0, None)],
        );
        let mut current = Snapshot::error("codex", "Codex", "timeout".into());

        assert!(!restore_last_success_after_error(
            &mut current,
            &previous,
            SNAPSHOT_CACHE_MS + 1,
        ));
        assert_eq!(current.status, "error");
        assert!(!current.stale);
    }

    #[test]
    fn fold_keeps_moonshot_when_kimi_has_no_wallet() {
        let mut all = vec![
            Snapshot::ok(
                "kimi",
                "Kimi Code",
                None,
                vec![Metric::progress("Session", 0.0, None)],
            ),
            Snapshot::ok(
                "moonshot",
                "Kimi API",
                None,
                vec![Metric::progress("Credits used", 24.0, None)],
            ),
        ];
        fold_moonshot_into_kimi(&mut all);
        assert!(all.iter().any(|s| s.id == "moonshot"));
    }

    #[test]
    fn fold_hides_moonshot_when_kimi_has_wallet_or_moonshot_is_empty() {
        let mut with_api = vec![
            Snapshot::ok(
                "kimi",
                "Kimi Code",
                None,
                vec![
                    Metric::progress("Session", 0.0, None),
                    Metric::progress("API", 24.0, None),
                ],
            ),
            Snapshot::ok(
                "moonshot",
                "Kimi API",
                None,
                vec![Metric::progress("Credits used", 24.0, None)],
            ),
        ];
        fold_moonshot_into_kimi(&mut with_api);
        assert!(!with_api.iter().any(|s| s.id == "moonshot"));

        let mut empty_moon = vec![
            Snapshot::ok(
                "kimi",
                "Kimi Code",
                None,
                vec![Metric::progress("Session", 0.0, None)],
            ),
            Snapshot::no_credentials("moonshot", "Kimi API", "paste a key"),
        ];
        fold_moonshot_into_kimi(&mut empty_moon);
        assert!(!empty_moon.iter().any(|s| s.id == "moonshot"));
    }







    struct TempConfig {
        dir: std::path::PathBuf,
    }

    impl TempConfig {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!(
                "pane-config-{}-{stamp}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }
    }

    impl Drop for TempConfig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn concurrent_config_patches_keep_both_updates() {
        let tmp = TempConfig::new();
        std::fs::write(tmp.dir.join("config.json"), "{}").unwrap();
        for round in 0..8 {
            std::fs::write(tmp.dir.join("config.json"), "{}").unwrap();
            let dir_a = tmp.dir.clone();
            let dir_b = tmp.dir.clone();
            let t1 = std::thread::spawn(move || set_config_in(&dir_a, json!({ "disabled": ["claude"] })));
            let t2 = std::thread::spawn(move || set_config_in(&dir_b, json!({ "locale": "zh" })));
            t1.join()
                .expect("disabled patch thread")
                .unwrap_or_else(|e| panic!("round {round} disabled patch: {e}"));
            t2.join()
                .expect("locale patch thread")
                .unwrap_or_else(|e| panic!("round {round} locale patch: {e}"));
            let cfg = load_config_from(&tmp.dir);
            assert_eq!(
                cfg["disabled"],
                json!(["claude"]),
                "round {round} lost disabled patch"
            );
            assert_eq!(cfg["locale"], json!("zh"), "round {round} lost locale patch");
        }
    }

    fn no_tmp_leftovers(dir: &std::path::Path) -> bool {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp"))
    }

    fn fail_write(_path: &std::path::Path, _raw: &str) -> std::io::Result<()> {
        Err(std::io::Error::other("disk full"))
    }

    fn partial_write(path: &std::path::Path, raw: &str) -> std::io::Result<()> {
        std::fs::write(path, &raw[..raw.len() / 2])?;
        Err(std::io::Error::other("disk full"))
    }

    fn fail_replace(_tmp: &std::path::Path, _path: &std::path::Path) -> std::io::Result<()> {
        Err(std::io::Error::other("file in use"))
    }

    #[test]
    fn non_object_main_config_does_not_clobber_or_lose_the_backup() {
        let tmp = TempConfig::new();
        let good = json!({"locale": "zh"});
        std::fs::write(
            tmp.dir.join("config.json.bak"),
            serde_json::to_string(&good).unwrap(),
        )
        .unwrap();
        std::fs::write(tmp.dir.join("config.json"), "[]").unwrap();

        set_config_in(&tmp.dir, json!({ "density": "compact" })).unwrap();

        let backup: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.dir.join("config.json.bak")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            backup["locale"], "zh",
            "valid non-object JSON must not overwrite the object backup"
        );
        let main = load_config_from(&tmp.dir);
        assert_eq!(main["density"], "compact");
        assert_eq!(
            main["locale"], "zh",
            "the save must patch the recovered backup, not empty defaults"
        );
    }

    #[test]
    fn corrupt_main_config_never_clobbers_the_good_backup() {
        let tmp = TempConfig::new();
        let good = json!({"locale": "zh"});
        std::fs::write(
            tmp.dir.join("config.json.bak"),
            serde_json::to_string(&good).unwrap(),
        )
        .unwrap();
        std::fs::write(tmp.dir.join("config.json"), "{corrupt").unwrap();

        set_config_in(&tmp.dir, json!({ "density": "compact" })).unwrap();

        let backup: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.dir.join("config.json.bak")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            backup, good,
            "a corrupt main file must never overwrite the good backup"
        );
        let main = load_config_from(&tmp.dir);
        assert_eq!(main["density"], "compact");
        assert_eq!(
            main["locale"], "zh",
            "the save patches on top of the recovered backup, not defaults"
        );
    }

    #[test]
    fn failed_temp_write_leaves_main_and_backup_untouched() {
        let tmp = TempConfig::new();
        std::fs::write(tmp.dir.join("config.json"), r#"{"locale":"zh"}"#).unwrap();
        let error = persist_config_at(
            &tmp.dir,
            &json!({"density": "compact"}),
            ConfigPersistIo {
                write_tmp: fail_write,
                ..ConfigPersistIo::real()
            },
        )
        .unwrap_err();
        assert!(error.contains("write config"), "{error}");
        assert_eq!(load_config_from(&tmp.dir)["locale"], "zh");
        assert!(!tmp.dir.join("config.json.bak").exists());
        assert!(no_tmp_leftovers(&tmp.dir));
    }

    #[test]
    fn partial_temp_write_is_cleaned_up() {
        let tmp = TempConfig::new();
        std::fs::write(tmp.dir.join("config.json"), "{}").unwrap();
        persist_config_at(
            &tmp.dir,
            &json!({"density": "compact"}),
            ConfigPersistIo {
                write_tmp: partial_write,
                ..ConfigPersistIo::real()
            },
        )
        .unwrap_err();
        assert!(
            no_tmp_leftovers(&tmp.dir),
            "a half-written temp file must be removed"
        );
    }

    #[test]
    fn failed_replace_keeps_a_recoverable_config() {
        let tmp = TempConfig::new();
        std::fs::write(tmp.dir.join("config.json"), r#"{"locale":"zh"}"#).unwrap();
        let error = persist_config_at(
            &tmp.dir,
            &json!({"density": "compact"}),
            ConfigPersistIo {
                replace: fail_replace,
                ..ConfigPersistIo::real()
            },
        )
        .unwrap_err();
        assert!(error.contains("replace config"), "{error}");
        assert_eq!(load_config_from(&tmp.dir)["locale"], "zh");
        let backup: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.dir.join("config.json.bak")).unwrap(),
        )
        .unwrap();
        assert_eq!(backup["locale"], "zh");
        assert!(no_tmp_leftovers(&tmp.dir));
    }

    #[test]
    fn backup_update_failure_does_not_block_the_save() {
        let tmp = TempConfig::new();
        std::fs::write(tmp.dir.join("config.json"), r#"{"locale":"zh"}"#).unwrap();
        // An unusable backup target (a directory) must not lose the save.
        std::fs::create_dir_all(tmp.dir.join("config.json.bak")).unwrap();
        set_config_in(&tmp.dir, json!({ "density": "compact" })).unwrap();
        let main = load_config_from(&tmp.dir);
        assert_eq!(main["density"], "compact");
        assert_eq!(main["locale"], "zh");
    }

    #[test]
    fn repeated_saves_keep_the_previous_valid_config_recoverable() {
        let tmp = TempConfig::new();
        set_config_in(&tmp.dir, json!({ "locale": "zh" })).unwrap();
        set_config_in(&tmp.dir, json!({ "density": "regular" })).unwrap();
        // Crash mid-write: the main file becomes garbage.
        std::fs::write(tmp.dir.join("config.json"), "{corrupt").unwrap();
        let recovered = load_config_from(&tmp.dir);
        assert_eq!(
            recovered["locale"], "zh",
            "the backup still loads the previous valid config"
        );
        // Saving from the recovered state heals the main file — without
        // letting the corrupt copy destroy the backup first.
        set_config_in(&tmp.dir, json!({ "spendTab": "week" })).unwrap();
        let healed = load_config_from(&tmp.dir);
        assert_eq!(healed["spendTab"], "week");
        assert_eq!(healed["locale"], "zh");
    }







    struct SnapCacheGuard(String);

    impl SnapCacheGuard {
        fn new(id: &str) -> Self {
            Self(id.to_string())
        }
    }

    impl Drop for SnapCacheGuard {
        fn drop(&mut self) {
            fail_state().lock().unwrap().remove(&self.0);
            last_ok().lock().unwrap().remove(&self.0);
        }
    }

    #[cfg(windows)]
    fn hold_no_delete(path: &std::path::Path) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1) // FILE_SHARE_READ
            .open(path)
            .expect("open held key file")
    }

    fn seed_cached_ok(id: &str, name: &str) {
        last_ok().lock().unwrap().insert(
            id.into(),
            CachedSnap {
                at: 1_000,
                snap: Snapshot::ok(
                    id,
                    name,
                    None,
                    vec![Metric::progress("Credits used", 80.0, None)],
                ),
            },
        );
    }

    #[test]
    fn api_key_context_scopes_only_key_backed_families() {
        assert!(is_credential_scoped_card("deepseek"));
        assert!(is_credential_scoped_card("sub2api@k1"));
        assert!(is_credential_scoped_card("onenewapi@k1"));
        assert!(!is_credential_scoped_card("claude"));
        assert!(!is_credential_scoped_card("claude@abcd1234"));
        assert!(!is_credential_scoped_card("codex"));
        assert!(!is_credential_scoped_card("grok"));
        assert!(!is_plain_api_key_provider("claude"));
        assert!(is_plain_api_key_provider("kimi"));
    }

    #[test]
    fn current_generations_cover_plain_providers_like_the_expected_side() {
        let _deepseek = SnapCacheGuard::new("deepseek");
        let batch = vec![
            Snapshot::ok("deepseek", "DeepSeek", None, vec![]),
            Snapshot::ok("claude", "Claude", None, vec![]),
        ];
        let current = current_credential_scoped_generations(&batch);
        let expected = key_card_snapshot_generations(["deepseek".to_string()]);
        assert_eq!(
            current.get("deepseek"),
            expected.get("deepseek"),
            "no rotation must compare equal and survive the retain"
        );
        assert!(!current.contains_key("claude"));

        let mut all = batch;
        let stale = retain_current_key_card_results(&mut all, &expected, &current);
        assert!(stale.is_empty(), "nothing dropped without a rotation");
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn rotating_plain_api_key_clears_only_that_providers_old_state() {
        let tmp = TempConfig::new();
        let rotated = "deepseek";
        let bystander = "aihubmix";
        let _rotated = SnapCacheGuard::new(rotated);
        let _bystander = SnapCacheGuard::new(bystander);
        set_api_key_in(&tmp.dir, rotated, "key-a").unwrap();
        set_api_key_in(&tmp.dir, bystander, "key-z").unwrap();
        seed_cached_ok(rotated, "DeepSeek");
        fail_state().lock().unwrap().insert(
            rotated.into(),
            FailState {
                observed_error: None,
                until_ms: i64::MAX,
                note: "HTTP 429 rate limited".into(),
            },
        );
        alerts::insert_state_for_test(&format!("{rotated}:Credits used"));
        seed_cached_ok(bystander, "AihubMix");

        set_api_key_in(&tmp.dir, rotated, "key-b").unwrap();

        {
            let cache = last_ok().lock().unwrap();
            assert!(
                !cache.contains_key(rotated),
                "the old account's success snapshot must not survive a rotation"
            );
            assert!(
                cache.contains_key(bystander),
                "an untouched provider keeps its cache"
            );
        }
        assert!(fail_state().lock().unwrap().get(rotated).is_none());
        assert!(!alerts::has_state_for_test(&format!(
            "{rotated}:Credits used"
        )));
        assert_eq!(
            stored_pane_api_key(&tmp.dir.join(format!("{rotated}.json"))).as_deref(),
            Some("key-b")
        );
        let before = key_card_snapshot_generations([rotated.to_string()]);
        set_api_key_in(&tmp.dir, rotated, "key-b").unwrap();
        let after = key_card_snapshot_generations([rotated.to_string()]);
        assert_eq!(before.get(rotated), after.get(rotated));
    }

    #[test]
    fn changing_minimax_key_forgets_remembered_tier() {
        let tmp = TempConfig::new();
        let _minimax = SnapCacheGuard::new("minimax");
        std::fs::write(
            tmp.dir.join("minimax-plan.json"),
            serde_json::json!({
                "tier": "Ultra Plan",
                "seen_ms": chrono::Utc::now().timestamp_millis(),
                "user_id": "1",
            })
            .to_string(),
        )
        .unwrap();

        set_api_key_in(&tmp.dir, "minimax", "sk-new-key-xxxxxxxx").unwrap();

        assert!(
            !tmp.dir.join("minimax-plan.json").exists(),
            "a pasted key must not inherit another account's remembered tier"
        );
    }

    #[test]
    fn resaving_same_minimax_key_keeps_remembered_tier() {
        let tmp = TempConfig::new();
        let _minimax = SnapCacheGuard::new("minimax");
        set_api_key_in(&tmp.dir, "minimax", "sk-same-key-xxxxxxxx").unwrap();
        std::fs::write(
            tmp.dir.join("minimax-plan.json"),
            serde_json::json!({
                "tier": "Ultra Plan",
                "seen_ms": chrono::Utc::now().timestamp_millis(),
                "user_id": "1",
            })
            .to_string(),
        )
        .unwrap();

        set_api_key_in(&tmp.dir, "minimax", "sk-same-key-xxxxxxxx").unwrap();

        assert!(
            tmp.dir.join("minimax-plan.json").exists(),
            "re-saving an unchanged key must not drop the remembered tier"
        );
    }

    #[test]
    fn clearing_minimax_key_forgets_remembered_tier() {
        let tmp = TempConfig::new();
        let _minimax = SnapCacheGuard::new("minimax");
        // No key file at all — the plan cache alone still triggers the
        // cleanup when the user hits Save with an empty field.
        std::fs::write(
            tmp.dir.join("minimax-plan.json"),
            serde_json::json!({
                "tier": "Ultra Plan",
                "seen_ms": chrono::Utc::now().timestamp_millis(),
                "user_id": "1",
            })
            .to_string(),
        )
        .unwrap();

        set_api_key_in(&tmp.dir, "minimax", "").unwrap();

        assert!(
            !tmp.dir.join("minimax-plan.json").exists(),
            "clearing the key must drop the remembered tier"
        );
    }

    #[test]
    fn minimax_key_change_is_refused_while_stale_tier_cannot_be_removed() {
        let tmp = TempConfig::new();
        let _minimax = SnapCacheGuard::new("minimax");
        set_api_key_in(&tmp.dir, "minimax", "sk-a-xxxxxxxxxx").unwrap();

        // A directory where the STASH file belongs: remove_file can't
        // clear it, then the rename onto it fails — so the new key must
        // not be saved while the old tier survives.
        std::fs::write(
            tmp.dir.join("minimax-plan.json"),
            serde_json::json!({
                "tier": "Ultra Plan",
                "seen_ms": chrono::Utc::now().timestamp_millis(),
                "user_id": "1",
            })
            .to_string(),
        )
        .unwrap();
        std::fs::create_dir(tmp.dir.join("minimax-plan.json.old")).unwrap();
        set_api_key_in(&tmp.dir, "minimax", "sk-b-xxxxxxxxxx")
            .expect_err("a failed tier stash must refuse the key change");
        assert_eq!(
            stored_pane_api_key(&tmp.dir.join("minimax.json")).as_deref(),
            Some("sk-a-xxxxxxxxxx"),
            "the old key stays so the retry still sees a rotation"
        );
        assert!(tmp.dir.join("minimax-plan.json").exists());

        std::fs::remove_dir(tmp.dir.join("minimax-plan.json.old")).unwrap();
        set_api_key_in(&tmp.dir, "minimax", "sk-b-xxxxxxxxxx").unwrap();
        assert_eq!(
            stored_pane_api_key(&tmp.dir.join("minimax.json")).as_deref(),
            Some("sk-b-xxxxxxxxxx")
        );
        assert!(!tmp.dir.join("minimax-plan.json").exists());
        assert!(!tmp.dir.join("minimax-plan.json.old").exists());
    }

    #[test]
    fn failed_minimax_key_write_restores_remembered_tier() {
        let tmp = TempConfig::new();
        let _minimax = SnapCacheGuard::new("minimax");
        set_api_key_in(&tmp.dir, "minimax", "sk-a-xxxxxxxxxx").unwrap();
        let plan_json = serde_json::json!({
            "tier": "Ultra Plan",
            "seen_ms": chrono::Utc::now().timestamp_millis(),
            "user_id": "1",
        })
        .to_string();
        std::fs::write(tmp.dir.join("minimax-plan.json"), &plan_json).unwrap();

        // A directory where the key file belongs makes atomic_write fail —
        // the stash must go back so the surviving old key keeps its tier.
        std::fs::remove_file(tmp.dir.join("minimax.json")).unwrap();
        std::fs::create_dir(tmp.dir.join("minimax.json")).unwrap();
        set_api_key_in(&tmp.dir, "minimax", "sk-b-xxxxxxxxxx")
            .expect_err("the key write must fail against a directory");
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("minimax-plan.json")).unwrap(),
            plan_json,
            "a failed key write restores the stashed tier"
        );
        assert!(!tmp.dir.join("minimax-plan.json.old").exists());
    }

    /// rotating_moonshot… and rotating_kimi… exercise the same two global
    /// ids; under a parallel test runner they'd evict each other's
    /// fixtures, so they serialize on this lock.
    fn kimi_moonshot_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap()
    }

    #[test]
    fn rotating_moonshot_also_forgets_folded_kimi_wallet() {
        let _km = kimi_moonshot_test_lock();
        let tmp = TempConfig::new();
        let _moonshot = SnapCacheGuard::new("moonshot");
        let _kimi = SnapCacheGuard::new("kimi");
        set_api_key_in(&tmp.dir, "moonshot", "key-a").unwrap();
        seed_cached_ok("moonshot", "Kimi API");
        last_ok().lock().unwrap().insert(
            "kimi".into(),
            CachedSnap {
                at: 1_000,
                snap: Snapshot::ok(
                    "kimi",
                    "Kimi Code",
                    None,
                    vec![
                        Metric::progress("Session", 10.0, None),
                        Metric::progress("Credits used", 80.0, None),
                    ],
                ),
            },
        );
        alerts::insert_state_for_test("kimi:Credits used");

        set_api_key_in(&tmp.dir, "moonshot", "key-b").unwrap();

        let cache = last_ok().lock().unwrap();
        assert!(
            !cache.contains_key("moonshot"),
            "rotated Moonshot snapshot must go"
        );
        assert!(
            !cache.contains_key("kimi"),
            "folded Kimi wallet rows must not survive a Moonshot rotation"
        );
        assert!(!alerts::has_state_for_test("kimi:Credits used"));
    }

    #[test]
    fn rotating_deepseek_drops_the_old_credit_baseline() {
        let tmp = TempConfig::new();
        let _guard = SnapCacheGuard::new("deepseek");
        set_api_key_in(&tmp.dir, "deepseek", "key-a").unwrap();
        std::fs::write(
            tmp.dir.join("credit_baselines.json"),
            r#"{"deepseek": 40.0, "moonshot": 12.0}"#,
        )
        .unwrap();

        set_api_key_in(&tmp.dir, "deepseek", "key-b").unwrap();

        let doc: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.dir.join("credit_baselines.json")).unwrap(),
        )
        .unwrap();
        assert!(
            doc.get("deepseek").is_none(),
            "the new key must not inherit the old high-water mark"
        );
        assert_eq!(
            doc["moonshot"], 12.0,
            "an untouched provider keeps its baseline"
        );
    }

    #[test]
    fn rotating_kimi_does_not_reset_moonshot_usage() {
        let _km = kimi_moonshot_test_lock();
        let tmp = TempConfig::new();
        let _kimi = SnapCacheGuard::new("kimi");
        let _moonshot = SnapCacheGuard::new("moonshot");
        set_api_key_in(&tmp.dir, "kimi", "kimi-a").unwrap();
        set_api_key_in(&tmp.dir, "moonshot", "ms-a").unwrap();
        seed_cached_ok("kimi", "Kimi Code");
        seed_cached_ok("moonshot", "Kimi API");
        std::fs::write(
            tmp.dir.join("credit_baselines.json"),
            r#"{"kimi": 5.0, "moonshot": 12.0}"#,
        )
        .unwrap();

        set_api_key_in(&tmp.dir, "kimi", "kimi-b").unwrap();

        let cache = last_ok().lock().unwrap();
        assert!(
            !cache.contains_key("kimi"),
            "rotated Kimi snapshot must go"
        );
        assert!(
            cache.contains_key("moonshot"),
            "Moonshot's key did not change, so its snapshot and meter stay"
        );
        let doc: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.dir.join("credit_baselines.json")).unwrap(),
        )
        .unwrap();
        assert!(doc.get("kimi").is_none());
        assert_eq!(doc["moonshot"], 12.0);
    }

    #[test]
    fn leftover_cache_is_cleared_when_retrying_the_same_key() {
        let tmp = TempConfig::new();
        let id = "openrouter";
        let _guard = SnapCacheGuard::new(id);
        set_api_key_in(&tmp.dir, id, "key-b").unwrap();
        seed_cached_ok(id, "OpenRouter");
        fail_state().lock().unwrap().insert(
            id.into(),
            FailState {
                observed_error: None,
                until_ms: i64::MAX,
                note: "HTTP 429 rate limited".into(),
            },
        );

        set_api_key_in(&tmp.dir, id, "key-b").unwrap();

        assert!(
            !last_ok().lock().unwrap().contains_key(id),
            "retrying the already-saved key must finish a leftover cleanup"
        );
        assert!(fail_state().lock().unwrap().get(id).is_none());
    }

    #[test]
    fn blocked_credit_baselines_file_is_reported() {
        let tmp = TempConfig::new();
        // "qwen" belongs to clearing_missing_key…, which asserts that a
        // no-op clear leaves the generation untouched — this test bumps
        // its id's generation, so they must not share one.
        let id = "aihubmix";
        let _guard = SnapCacheGuard::new(id);
        set_api_key_in(&tmp.dir, id, "key-a").unwrap();
        std::fs::create_dir(tmp.dir.join("credit_baselines.json")).unwrap();
        let error = set_api_key_in(&tmp.dir, id, "key-b").expect_err("baseline IO must surface");
        assert!(
            error.contains("credit baselines") || error.contains("cached data"),
            "a baseline rewrite failure must not report success: {error}"
        );
        assert_eq!(
            stored_pane_api_key(&tmp.dir.join(format!("{id}.json"))).as_deref(),
            Some("key-b")
        );
    }

    #[test]
    fn failed_snapshot_persist_clears_memory_and_is_retryable() {
        let tmp = TempConfig::new();
        let id = "deepseek";
        let _guard = SnapCacheGuard::new(id);
        struct PersistFailGuard;
        impl Drop for PersistFailGuard {
            fn drop(&mut self) {
                TEST_PERSIST_LAST_OK_FAIL.with(|fail| fail.set(false));
                SNAPSHOT_CACHE_NEEDS_FLUSH.store(false, Ordering::Release);
            }
        }
        let _persist_guard = PersistFailGuard;
        set_api_key_in(&tmp.dir, id, "key-a").unwrap();
        seed_cached_ok(id, "DeepSeek");
        TEST_PERSIST_LAST_OK_FAIL.with(|fail| fail.set(true));
        let error = set_api_key_in(&tmp.dir, id, "key-b").expect_err("persist fail must surface");
        assert!(
            error.contains("cached data"),
            "cleanup failure must not look like a successful save: {error}"
        );
        assert_eq!(
            stored_pane_api_key(&tmp.dir.join(format!("{id}.json"))).as_deref(),
            Some("key-b")
        );
        assert!(
            !last_ok().lock().unwrap().contains_key(id),
            "memory must drop the old snapshot even when disk persist fails"
        );
        set_api_key_in(&tmp.dir, id, "key-b").unwrap();
        assert!(!last_ok().lock().unwrap().contains_key(id));
        assert!(!SNAPSHOT_CACHE_NEEDS_FLUSH.load(Ordering::Acquire));
    }

    #[test]
    fn rotated_key_late_result_is_refused_and_401_shows_no_old_data() {
        let tmp = TempConfig::new();
        let id = "openrouter";
        let _guard = SnapCacheGuard::new(id);
        set_api_key_in(&tmp.dir, id, "key-a").unwrap();
        let expected = key_card_snapshot_generations([id.to_string()]);
        set_api_key_in(&tmp.dir, id, "key-b").unwrap();
        assert!(!last_ok().lock().unwrap().contains_key(id));
        let mut publishable = vec![
            Snapshot::ok(id, "OpenRouter", None, vec![]),
            Snapshot::ok("claude", "Claude", None, vec![]),
        ];
        let current = current_credential_scoped_generations(&publishable);
        let stale = retain_current_key_card_results(&mut publishable, &expected, &current);
        assert_eq!(stale, vec![id.to_string()]);
        assert_eq!(publishable.len(), 1);
        assert_eq!(publishable[0].id, "claude");
    }

    #[test]
    fn late_failure_after_key_rotation_cannot_bench_the_new_context() {
        let id = "kilo";
        let _guard = SnapCacheGuard::new(id);
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let fut = async move {
            started_tx.send(()).expect("test gate open");
            release_rx.await.expect("test release");
            Snapshot::error(id, "Kilo", "HTTP 429 rate limited".into())
        };
        let handle = std::thread::spawn(move || {
            tauri::async_runtime::block_on(guarded(id.to_string(), "Kilo".into(), fut))
        });
        started_rx.recv().unwrap();
        drop(KeyCardMutationGuard::begin(vec![id.to_string()]));
        release_tx.send(()).unwrap();
        let snap = handle.join().expect("guarded thread");
        assert_eq!(snap.status, "error");
        assert!(
            fail_state().lock().unwrap().get(id).is_none(),
            "a late failure from the old key must not bench the new key"
        );
    }

    #[test]
    fn fresh_failure_without_rotation_still_benches() {
        let id = "kilo-control";
        let _guard = SnapCacheGuard::new(id);
        let snap = tauri::async_runtime::block_on(guarded(id.to_string(), "Kilo".into(), async {
            Snapshot::error(id, "Kilo", "HTTP 429 rate limited".into())
        }));
        assert_eq!(snap.status, "error");
        assert!(fail_state().lock().unwrap().contains_key(id));
    }

    #[test]
    fn clearing_missing_key_file_succeeds_and_changes_nothing() {
        let tmp = TempConfig::new();
        let id = "qwen";
        let _guard = SnapCacheGuard::new(id);
        let before = key_card_snapshot_generations([id.to_string()]);
        set_api_key_in(&tmp.dir, id, "").unwrap();
        let after = key_card_snapshot_generations([id.to_string()]);
        assert_eq!(before.get(id), after.get(id));
        assert!(!tmp.dir.join(format!("{id}.json")).exists());
    }

    #[cfg(windows)]
    #[test]
    fn clearing_blocked_key_file_reports_failure_and_keeps_state() {
        let tmp = TempConfig::new();
        let id = "zai";
        let _guard = SnapCacheGuard::new(id);
        set_api_key_in(&tmp.dir, id, "key-a").unwrap();
        seed_cached_ok(id, "Z.ai");
        let held = hold_no_delete(&tmp.dir.join(format!("{id}.json")));
        let result = set_api_key_in(&tmp.dir, id, "");
        drop(held);
        let error = result.expect_err("a locked key file must not report success");
        assert!(error.contains("remove key file"), "{error}");
        assert!(stored_pane_api_key(&tmp.dir.join(format!("{id}.json"))).is_some());
        assert!(last_ok().lock().unwrap().contains_key(id));
    }

    #[cfg(windows)]
    #[test]
    fn failed_key_save_keeps_the_old_credential_and_state() {
        let tmp = TempConfig::new();
        let id = "minimax";
        let _guard = SnapCacheGuard::new(id);
        set_api_key_in(&tmp.dir, id, "key-a").unwrap();
        seed_cached_ok(id, "MiniMax");
        let expected = key_card_snapshot_generations([id.to_string()]);
        let held = hold_no_delete(&tmp.dir.join(format!("{id}.json")));
        let result = set_api_key_in(&tmp.dir, id, "key-b");
        drop(held);
        assert!(result.is_err(), "a failed write must not report success");
        assert_eq!(
            stored_pane_api_key(&tmp.dir.join(format!("{id}.json"))).as_deref(),
            Some("key-a")
        );
        assert!(last_ok().lock().unwrap().contains_key(id));
        let after = key_card_snapshot_generations([id.to_string()]);
        assert_eq!(expected.get(id), after.get(id));
    }

    #[test]
    fn forget_provider_snapshot_clears_fail_state_and_last_ok() {
        let id = "onenewapi@ticket03-forget";
        let _guard = SnapCacheGuard::new(id);
        fail_state().lock().unwrap().insert(
            id.to_string(),
            FailState {
                observed_error: None,
                until_ms: i64::MAX,
                note: "benched".into(),
            },
        );
        last_ok().lock().unwrap().insert(
            id.to_string(),
            CachedSnap {
                at: 1,
                snap: Snapshot::ok(
                    id,
                    "Panel · Old",
                    None,
                    vec![Metric::text("Limit", "$10.00".into())],
                ),
            },
        );
        forget_provider_snapshot(id).unwrap();
        assert!(!fail_state().lock().unwrap().contains_key(id));
        assert!(!last_ok().lock().unwrap().contains_key(id));
    }

    #[test]
    fn rename_cached_snapshot_updates_name_only() {
        let id = "onenewapi@ticket03-rename";
        let _guard = SnapCacheGuard::new(id);
        fail_state().lock().unwrap().insert(
            id.to_string(),
            FailState {
                observed_error: None,
                until_ms: i64::MAX,
                note: "benched".into(),
            },
        );
        last_ok().lock().unwrap().insert(
            id.to_string(),
            CachedSnap {
                at: 42,
                snap: Snapshot::ok(
                    id,
                    "Panel · Old",
                    None,
                    vec![Metric::text("Limit", "$10.00".into())],
                ),
            },
        );
        rename_cached_snapshot(id, "Panel · New".into()).unwrap();
        let map = last_ok().lock().unwrap();
        let entry = map.get(id).unwrap();
        assert_eq!(entry.snap.name, "Panel · New");
        assert_eq!(entry.at, 42);
        assert_eq!(entry.snap.status, "ok");
        assert_eq!(entry.snap.metrics.len(), 1);
        assert_eq!(entry.snap.metrics[0].label, "Limit");
        assert_eq!(entry.snap.metrics[0].value.as_deref(), Some("$10.00"));
        drop(map);
        assert_eq!(
            fail_state().lock().unwrap().get(id).unwrap().note,
            "benched"
        );
    }















    fn sample_card_layout() -> Value {
        json!({
            "metricOrder": ["Usage"],
            "onDemand": [],
            "hidden": [],
            "starred": ["Usage"],
            "expanded": false
        })
    }























    #[test]
    fn tray_digits_double_rows_keep_three_transparent_lines() {
        let rgba = super::draw_tray_numbers_with(&[100, 97], super::TRAY_INK_FALLBACK);
        for y in 14..=16 {
            for x in 0..32 {
                let index = (y * 32 + x) * 4;
                assert_eq!(&rgba[index..index + 4], &[0, 0, 0, 0], "x={x} y={y}");
            }
        }
    }

    #[test]
    fn tray_digits_light_palette_is_exact() {
        let rgba = super::draw_tray_numbers_with(&[100, 97], super::TRAY_INK_LIGHT);
        let foreground = (1 * 32 + 6) * 4;
        let outline = (1 * 32 + 5) * 4;
        assert_eq!(&rgba[foreground..foreground + 4], &[20, 24, 33, 255]);
        assert_eq!(&rgba[outline..outline + 4], &[255, 255, 255, 220]);
    }

    #[test]
    fn tray_digits_dark_and_fallback_palettes() {
        let pixel = |ink| {
            let rgba = super::draw_tray_numbers_with(&[100], ink);
            (rgba[(10 * 32 + 6) * 4..(10 * 32 + 6) * 4 + 4].to_vec(),
             rgba[(10 * 32 + 5) * 4..(10 * 32 + 5) * 4 + 4].to_vec())
        };
        assert_eq!(pixel(super::tray_ink_for_theme(Some(0))), (vec![255, 255, 255, 255], vec![0, 0, 0, 200]));
        assert_eq!(pixel(super::tray_ink_for_theme(None)), (vec![255, 255, 255, 255], vec![0, 0, 0, 230]));
        assert_eq!(pixel(super::tray_ink_for_theme(Some(2))), pixel(super::TRAY_INK_FALLBACK));
    }

    #[test]
    fn tray_digits_keep_edges_and_empty_image_clear() {
        let rgba = super::draw_tray_numbers_with(&[100, 100], super::TRAY_INK_DARK);
        for y in 0..32 {
            for x in [0, 31] {
                assert_eq!(&rgba[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4], &[0, 0, 0, 0]);
            }
        }
        assert!(super::draw_tray_numbers_with(&[], super::TRAY_INK_DARK)
            .iter().all(|channel| *channel == 0));
        let single = super::draw_tray_numbers_with(&[97], super::TRAY_INK_DARK);
        for y in 0..32 {
            let has_foreground = (0..32).any(|x| &single[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4] == &[255, 255, 255, 255]);
            assert_eq!(has_foreground, (10..=21).contains(&y), "y={y}");
        }
    }

    #[test]
    fn tray_popover_bottom_ignores_click_height_and_offsets_outer_frame() {
        use super::{popover_origin, popover_outer_origin, ScreenRect, WindowInsets};
        let monitor = ScreenRect { x: 0, y: 0, width: 1920, height: 1080 };
        let work = ScreenRect { x: 0, y: 0, width: 1920, height: 1040 };
        assert_eq!(popover_origin((1800, 1050), (380, 600), monitor, work), (1420, 440));
        assert_eq!(popover_origin((1800, 1070), (380, 600), monitor, work), (1420, 440));
        let insets = WindowInsets { left: 8, top: 1, right: 8, bottom: 8 };
        assert_eq!(popover_outer_origin((1800, 1070), (380, 600), monitor, work, insets), (1428, 448));
    }

    #[test]
    fn tray_popover_top_and_sides_follow_work_area() {
        use super::{popover_origin, ScreenRect};
        let monitor = ScreenRect { x: 0, y: 0, width: 1920, height: 1080 };
        let top = ScreenRect { x: 0, y: 48, width: 1920, height: 1032 };
        assert_eq!(popover_origin((1800, 1000), (380, 600), monitor, top), (1420, 48));
        let left = ScreenRect { x: 48, y: 0, width: 1872, height: 1080 };
        assert_eq!(popover_origin((20, 950), (380, 600), monitor, left), (48, 350));
        let right = ScreenRect { x: 0, y: 0, width: 1872, height: 1080 };
        assert_eq!(popover_origin((1900, 100), (380, 600), monitor, right), (1492, 0));
    }

    #[test]
    fn tray_popover_ties_and_oversized_window_have_stable_origin() {
        use super::{popover_origin, ScreenRect};
        let monitor = ScreenRect { x: 0, y: 0, width: 1920, height: 1080 };
        let no_inset = monitor;
        assert_eq!(popover_origin((1800, 1000), (380, 600), monitor, no_inset), (1420, 480));
        let tie = ScreenRect { x: 0, y: 40, width: 1920, height: 1000 };
        assert_eq!(popover_origin((1800, 1000), (380, 600), monitor, tie), (1420, 440));
        let small = ScreenRect { x: 40, y: 50, width: 300, height: 200 };
        assert_eq!(popover_origin((200, 100), (380, 600), monitor, small), (40, 50));
        assert_eq!(popover_origin((1800, 1050), (380, 500), monitor, tie), (1420, 540));
    }
}
