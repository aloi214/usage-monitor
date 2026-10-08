use super::{http, stored_api_key, Metric, Snapshot};
use serde_json::Value;
use std::time::Duration;

const ID: &str = "moonshot";
const NAME: &str = "Kimi API";
const MAX_BALANCE_BYTES: usize = 64 * 1024;

pub async fn snapshot() -> Snapshot {
    match fetch().await {
        Ok(s) => s,
        Err(e) => Snapshot::error(ID, NAME, e),
    }
}

/// True when a Moonshot/Kimi API key is saved or in the environment.
/// Plan-only installs have none — the Kimi card then skips the API row.
pub fn has_api_key() -> bool {
    if crate::network_policy::current_selection(ID).is_err() { return false; }
    stored_api_key("moonshot", &["MOONSHOT_API_KEY", "KIMI_API_KEY"]).is_some()
}

/// Wallet fetch is allowed only with a saved key *and* Moonshot switched
/// on. A disabled Moonshot must not be contacted via the folded Kimi card.
pub fn wallet_wanted() -> bool {
    wallet_wanted_from(has_api_key(), super::provider_disabled("moonshot"))
}

fn wallet_wanted_from(has_key: bool, moonshot_disabled: bool) -> bool {
    has_key && !moonshot_disabled
}

/// Wallet rows for the Kimi Code card (Session / Weekly / API). Empty `Ok`
/// when no key is saved or Moonshot is off — the plan bars still stand on
/// their own. `Err` is a failed balance call the caller can warn on
/// without failing the plan.
pub async fn api_rows() -> Result<Vec<Metric>, String> {
    crate::network_policy::current_selection(ID)?;
    if !wallet_wanted() {
        return Ok(Vec::new());
    }
    fetch_balance(true).await
}

async fn fetch() -> Result<Snapshot, String> {
    crate::network_policy::current_selection(ID)?;
    if stored_api_key("moonshot", &["MOONSHOT_API_KEY", "KIMI_API_KEY"]).is_none() {
        return Ok(Snapshot::no_credentials(
            ID,
            NAME,
            "Paste a Kimi API key in Settings (gear icon).",
        ));
    }
    let rows = fetch_balance(false).await?;
    Ok(Snapshot::ok(ID, NAME, Some("Pay as you go".into()), rows))
}

async fn fetch_balance(api_label: bool) -> Result<Vec<Metric>, String> {
    let selection = crate::network_policy::current_selection(ID)?;
    let base = crate::network_policy::selected_origin(ID)?;
    let Some(key) = stored_api_key(ID, &["MOONSHOT_API_KEY", "KIMI_API_KEY"]) else {
        return Err("no key".into());
    };
    let resp = http(ID).get(format!("{base}/v1/users/me/balance"))
        .bearer_auth(&key).timeout(Duration::from_secs(8)).send().await
        .map_err(|e| format!("balance request: {e}"))?;
    if !resp.status().is_success() { return Err(format!("Selected region balance endpoint: HTTP {}. Check your region and key in Settings", resp.status())); }
    let doc: Value = super::json_body(resp, MAX_BALANCE_BYTES, "balance").await?;
    let data = doc.get("data").unwrap_or(&doc);
    let available = data.get("available_balance").and_then(Value::as_f64).ok_or("no balance in response")?;
    let voucher = data.get("voucher_balance").and_then(Value::as_f64);
    let cash = data.get("cash_balance").and_then(Value::as_f64);
    let sign = if selection.starts_with("china:") { "¥" } else { "$" };
    Ok(rows_from_balance(available, voucher, cash, sign, api_label))
}

fn rows_from_balance(
    available: f64,
    voucher: Option<f64>,
    cash: Option<f64>,
    sign: &str,
    api_label: bool,
) -> Vec<Metric> {
    let mut metrics = Vec::new();
    // High-water key stays "moonshot" so a fold onto the Kimi card keeps
    // the same Credits-used baseline the Moonshot card already learned.
    let label = wallet_label(api_label);
    if let Some(meter) = super::credit_meter_labeled(ID, sign, available, label, "") {
        metrics.push(meter);
    }
    metrics.extend(text_rows(available, voucher, cash, sign, api_label));
    metrics
}

fn wallet_label(api_label: bool) -> &'static str {
    if api_label {
        "API"
    } else {
        "Credits used"
    }
}

fn text_rows(
    available: f64,
    voucher: Option<f64>,
    cash: Option<f64>,
    sign: &str,
    api_label: bool,
) -> Vec<Metric> {
    let mut metrics = Vec::new();
    metrics.push(Metric::text("Balance", format!("{sign}{available:.2}")));
    if let Some(v) = voucher {
        if v > 0.0 {
            metrics.push(Metric::text("Vouchers", format!("{sign}{v:.2}")));
        }
    }
    if let Some(c) = cash {
        if !api_label || c > 0.0 {
            metrics.push(Metric::text("Cash", format!("{sign}{c:.2}")));
        }
    }
    metrics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kimi_fold_labels_the_wallet_api() {
        assert_eq!(wallet_label(true), "API");
        let rows = text_rows(80.0, Some(80.0), Some(0.0), "$", true);
        assert_eq!(
            rows.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(),
            ["Balance", "Vouchers"]
        );
    }

    #[test]
    fn standalone_moonshot_keeps_credits_used() {
        assert_eq!(wallet_label(false), "Credits used");
        let rows = text_rows(50.0, None, Some(0.0), "$", false);
        assert_eq!(
            rows.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(),
            ["Balance", "Cash"]
        );
    }

    #[test]
    fn wallet_stays_off_when_moonshot_is_disabled() {
        assert!(wallet_wanted_from(true, false));
        assert!(!wallet_wanted_from(true, true));
        assert!(!wallet_wanted_from(false, false));
        assert!(!wallet_wanted_from(false, true));
    }
}
