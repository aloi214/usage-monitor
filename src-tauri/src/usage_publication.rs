//! Portable publication boundary shared by IPC, HTTP reads, and native trays.
use crate::access_policy::{AccessPolicy, AccessRuntime};
use crate::snapshot::Snapshot;
use serde::Serialize;
use std::sync::Mutex;

#[derive(Clone, Serialize)]
pub struct UsageResult {
    pub revision: u64,
    pub snapshots: Vec<Snapshot>,
}
impl UsageResult {
    pub fn empty(revision: u64) -> Self {
        Self {
            revision,
            snapshots: Vec::new(),
        }
    }
}
#[derive(Clone)]
struct PublishedUsage {
    result: UsageResult,
    // Fallback for snapshots without an original fetched_at. This belongs to
    // the publication, not to the time a later HTTP client reads it.
    published_at_ms: i64,
}
#[derive(Default)]
pub struct UsagePublication {
    value: Mutex<Option<PublishedUsage>>,
}
impl UsagePublication {
    pub fn clear(&self) {
        *self.value.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    pub fn publish(
        &self,
        runtime: &AccessRuntime,
        revision: u64,
        snapshots: Vec<Snapshot>,
    ) -> Option<UsageResult> {
        self.commit(runtime, revision, snapshots, |_| ())
            .map(|(result, ())| result)
    }
    pub fn commit<T>(
        &self,
        runtime: &AccessRuntime,
        revision: u64,
        snapshots: Vec<Snapshot>,
        effect: impl FnOnce(&[Snapshot]) -> T,
    ) -> Option<(UsageResult, T)> {
        self.commit_scoped(runtime, revision, None, snapshots, effect)
    }
    /// Merge one account into the latest publication under the same policy and
    /// value locks as publication. Untouched errors and timestamps remain exact.
    pub fn commit_scoped<T>(
        &self,
        runtime: &AccessRuntime,
        revision: u64,
        account: Option<&str>,
        mut snapshots: Vec<Snapshot>,
        effect: impl FnOnce(&[Snapshot]) -> T,
    ) -> Option<(UsageResult, T)> {
        runtime.commit_if_current(revision, |policy| {
            let mut publication = self.value.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(id) = account {
                snapshots.retain(|s| s.id == id);
                if let Some(previous) = publication
                    .as_ref()
                    .filter(|p| p.result.revision == revision)
                {
                    let mut merged = previous.result.snapshots.clone();
                    for snapshot in snapshots {
                        if let Some(old) = merged.iter_mut().find(|old| old.id == snapshot.id) {
                            *old = snapshot;
                        } else {
                            merged.push(snapshot);
                        }
                    }
                    snapshots = merged;
                }
            }
            filter_snapshots(&mut snapshots, policy);
            let applied = effect(&snapshots);
            let result = UsageResult {
                revision,
                snapshots,
            };
            let published_at_ms = if account.is_some() {
                publication
                    .as_ref()
                    .map(|p| p.published_at_ms)
                    .unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
            } else {
                chrono::Utc::now().timestamp_millis()
            };
            *publication = Some(PublishedUsage {
                result: result.clone(),
                published_at_ms,
            });
            (result, applied)
        })
    }
    pub fn read(&self, runtime: &AccessRuntime) -> Option<UsageResult> {
        self.read_with_published_at(runtime)
            .map(|(result, _)| result)
    }
    pub fn read_with_published_at(&self, runtime: &AccessRuntime) -> Option<(UsageResult, i64)> {
        let revision = self
            .value
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()?
            .result
            .revision;
        runtime
            .commit_if_current(revision, |policy| {
                let value = self.filtered_publication(revision, policy)?;
                Some((value.result, value.published_at_ms))
            })
            .flatten()
    }
    // Clone the result and its timestamp together so concurrent publication at
    // the same authorization revision cannot pair unrelated data and times.
    fn filtered_publication(&self, revision: u64, policy: &AccessPolicy) -> Option<PublishedUsage> {
        let mut value = self
            .value
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()?;
        if value.result.revision != revision {
            return None;
        }
        filter_snapshots(&mut value.result.snapshots, policy);
        Some(value)
    }
    pub fn apply<T>(
        &self,
        runtime: &AccessRuntime,
        revision: u64,
        action: impl FnOnce(&[Snapshot], &AccessPolicy) -> T,
    ) -> Option<T> {
        runtime
            .commit_if_current(revision, |policy| {
                let value = self.filtered_publication(revision, policy)?;
                Some(action(&value.result.snapshots, policy))
            })
            .flatten()
    }
    pub fn retain(&self, keep: impl Fn(&str) -> bool) {
        if let Some(value) = self
            .value
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            value.result.snapshots.retain(|s| keep(&s.id));
        }
    }
    pub fn rename(&self, names: &std::collections::HashMap<String, String>) {
        if let Some(value) = self
            .value
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            for s in &mut value.result.snapshots {
                if let Some(name) = names.get(&s.id) {
                    s.name = name.clone();
                }
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct StripView {
    pub values: Vec<u32>,
    pub tooltip: String,
}
pub fn strip_view(
    snapshots: &[Snapshot],
    policy: &AccessPolicy,
    id: &str,
    labels: &[String],
) -> Option<StripView> {
    if !policy.allows_account(id) {
        return None;
    }
    let snap = snapshots.iter().find(|s| s.id == id && s.status == "ok")?;
    let metrics: Vec<_> = labels
        .iter()
        .take(2)
        .filter(|label| policy.allows_metric(id, label))
        .filter_map(|label| {
            snap.metrics
                .iter()
                .find(|m| &m.label == label && m.kind == "progress")
        })
        .collect();
    if metrics.is_empty() {
        return None;
    }
    let values: Vec<u32> = metrics
        .iter()
        .map(|m| {
            (100.0 - m.used_percent.unwrap_or(0.0))
                .clamp(0.0, 100.0)
                .round() as u32
        })
        .collect();
    let saved_wallet = metrics
        .iter()
        .any(|metric| snap.has_saved_wallet_metric(&metric.label));
    let name = if saved_wallet {
        format!("⚠ {} (saved wallet)", snap.name)
    } else if snap.stale {
        format!("⚠ {}", snap.name)
    } else {
        snap.name.clone()
    };
    let tooltip = format!(
        "{}\n{}",
        name,
        metrics
            .iter()
            .zip(&values)
            .map(|(m, v)| format!("{}: {v}% left", m.label))
            .collect::<Vec<_>>()
            .join("\n")
    );
    Some(StripView { values, tooltip })
}
pub async fn clear_both(
    first: impl std::future::Future<Output = Result<(), String>>,
    second: impl std::future::Future<Output = Result<(), String>>,
) -> Result<(), String> {
    let first = first.await;
    let second = second.await;
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(a), Ok(())) | (Ok(()), Err(a)) => Err(a),
        (Err(a), Err(b)) => Err(format!("{a}; {b}")),
    }
}

pub fn filter_snapshots(snapshots: &mut Vec<Snapshot>, policy: &AccessPolicy) {
    snapshots.retain(|s| policy.allows_snapshot(&s.id, s.plan.as_deref()));
    for snapshot in snapshots {
        filter_snapshot(snapshot, policy);
    }
}

/// A composite card never grants access to the account that owns saved rows.
/// Clear source metadata/errors together with revoked or malformed provenance.
pub fn filter_snapshot(snapshot: &mut Snapshot, policy: &AccessPolicy) {
    snapshot
        .metrics
        .retain(|m| policy.allows_metric(&snapshot.id, &m.label));
    if snapshot.id == "kimi" {
        let wallet_authorized = policy.allows_account("moonshot")
            && snapshot.wallet_history.as_ref().is_none_or(|history| {
                history.source_account_id == "moonshot"
                    && policy.allows_account(&history.source_account_id)
            });
        if !wallet_authorized {
            snapshot.metrics.retain(|m| {
                !matches!(
                    m.label.as_str(),
                    "API" | "Credits used" | "Balance" | "Vouchers" | "Cash"
                )
            });
            let wallet_warning = snapshot.warning.as_ref().is_some_and(|warning| {
                warning.starts_with("Moonshot API wallet")
                    || snapshot
                        .wallet_history
                        .as_ref()
                        .and_then(|history| history.warning.as_ref())
                        == Some(warning)
            });
            snapshot.wallet_history = None;
            // Remove only the source's own error. A failed Kimi plan query may
            // have replaced the top-level warning while retaining wallet rows.
            if wallet_warning {
                snapshot.warning = None;
            }
        }
    }
}
