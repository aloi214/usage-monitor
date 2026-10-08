//! Exact provider destinations. Configuration selects a mode; it cannot add hosts.
use crate::access_policy::{self, AccessPermit};
use reqwest::Url;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderEndpoint {
    pub provider: String,
    /// For regional providers this includes the explicitly selected credential kind.
    pub region: String,
    pub origin: String,
}

#[derive(Clone, Copy, Serialize)]
pub struct RegionChoice {
    pub value: &'static str,
    pub label: &'static str,
}
macro_rules! choices { ($($value:literal => $label:literal),* $(,)?) => { &[$(RegionChoice { value: $value, label: $label }),*] }; }
pub fn region_choices(provider: &str) -> &'static [RegionChoice] {
    match provider {
        "zai" | "moonshot" => {
            choices!["international:api_key" => "International · API key", "china:api_key" => "China · API key"]
        }
        "minimax" => {
            choices!["international:api_key" => "International · API key", "china:api_key" => "China · API key", "international:mcode" => "International · MiniMax Code sign-in", "china:mcode" => "China · MiniMax Code sign-in"]
        }
        "qwen" => {
            choices!["international:bearer" => "International · Bearer key", "china:bearer" => "China · Bearer key", "international:x_api_key" => "International · x-api-key", "china:x_api_key" => "China · x-api-key", "international:dashscope_api_key" => "International · x-dashscope-api-key", "china:dashscope_api_key" => "China · x-dashscope-api-key"]
        }
        "stepfun" => {
            choices!["international:api_key" => "International · Wallet API key", "china:api_key" => "China · Wallet API key", "international:plan_key" => "International · Step Plan key", "china:plan_key" => "China · Step Plan key"]
        }
        "antigravity" => {
            choices!["cloud" => "Google Cloud Code", "local_process" => "Local Antigravity process"]
        }
        "ollama" => {
            choices!["local:http://127.0.0.1:11434" => "Local IPv4 · port 11434", "local:http://[::1]:11434" => "Local IPv6 · port 11434", "local:http://localhost:11434" => "Localhost · port 11434"]
        }
        _ => &[],
    }
}

/// Validate before storing through set_provider_region. Ollama's custom port
/// editor must construct an origin and call this validator; no arbitrary host.
pub fn validate_selection(provider: &str, selection: &str) -> Result<(), String> {
    if provider == "ollama" {
        let origin = selection
            .strip_prefix("local:")
            .ok_or_else(configuration_required)?;
        return local_origin(origin).map(|_| ());
    }
    let choices = region_choices(provider);
    if !choices.is_empty() {
        if choices.iter().any(|c| c.value == selection) {
            return Ok(());
        }
        return Err(configuration_required());
    }
    if remote_origins(provider, "global").is_empty() || selection != "global" {
        return Err(configuration_required());
    }
    Ok(())
}
fn configuration_required() -> String {
    "Choose this account's region and credential type in Settings before querying it".into()
}

pub fn current_selection(provider: &str) -> Result<String, String> {
    let permit = access_policy::current_operation()
        .ok_or("Credential operation has no account authorization")?;
    selection_for_permit(provider, &permit)
}
pub(crate) fn selection_for_permit(
    provider: &str,
    permit: &AccessPermit,
) -> Result<String, String> {
    let policy = permit.policy_snapshot()?;
    let account = permit.account_id();
    let family = if account == provider {
        provider
    } else {
        policy
            .account_bindings
            .get(account)
            .map(|b| b.family.as_str())
            .ok_or("Account/provider binding is missing")?
    };
    if family != provider {
        return Err("The operation is authorized for a different provider".into());
    }
    let default = if provider == "antigravity" {
        "cloud"
    } else if region_choices(provider).is_empty() {
        "global"
    } else {
        ""
    };
    let selection = policy
        .regions
        .get(account)
        .map(String::as_str)
        .unwrap_or(default);
    validate_selection(provider, selection)?;
    Ok(selection.to_string())
}

pub fn remote_origins(provider: &str, selection: &str) -> &'static [&'static str] {
    match (provider, selection) {
        ("aihubmix", "global") => &["https://aihubmix.com"],
        ("antigravity", "cloud") => &[
            "https://oauth2.googleapis.com",
            "https://daily-cloudcode-pa.googleapis.com",
            "https://cloudcode-pa.googleapis.com",
        ],
        ("claude", "global") => &["https://api.anthropic.com", "https://platform.claude.com"],
        ("commandcode", "global") => &["https://api.commandcode.ai"],
        ("codebuff", "global") => &["https://www.codebuff.com"],
        ("codex", "global") => &["https://chatgpt.com", "https://auth.openai.com"],
        ("copilot", "global") => &["https://api.github.com"],
        ("cursor", "global") => &["https://api2.cursor.sh", "https://cursor.com"],
        ("deepseek", "global") => &["https://api.deepseek.com"],
        ("devin", "global") => &["https://server.codeium.com"],
        ("elevenlabs", "global") => &["https://api.elevenlabs.io"],
        ("grok", "global") => &[
            "https://auth.x.ai",
            "https://cli-chat-proxy.grok.com",
            "https://grok.com",
        ],
        ("kilo", "global") => &["https://app.kilo.ai"],
        ("kimi", "global") => &["https://api.kimi.com", "https://auth.kimi.com"],
        ("opencode", "global") => &["https://opencode.ai"],
        ("openrouter", "global") => &["https://openrouter.ai"],
        ("minimax", "international:api_key") => &["https://api.minimax.io"],
        ("minimax", "china:api_key") => &["https://api.minimaxi.com"],
        ("minimax", "international:mcode") => {
            &["https://agent.minimax.io", "https://platform.minimax.io"]
        }
        ("minimax", "china:mcode") => &["https://agent.minimaxi.com", "https://www.minimaxi.com"],
        ("moonshot", "international:api_key") => &["https://api.moonshot.ai"],
        ("moonshot", "china:api_key") => &["https://api.moonshot.cn"],
        (
            "qwen",
            "international:bearer" | "international:x_api_key" | "international:dashscope_api_key",
        ) => &["https://modelstudio.console.alibabacloud.com"],
        ("qwen", "china:bearer" | "china:x_api_key" | "china:dashscope_api_key") => {
            &["https://bailian.console.aliyun.com"]
        }
        ("stepfun", "international:api_key" | "international:plan_key") => {
            &["https://api.stepfun.ai"]
        }
        ("stepfun", "china:api_key" | "china:plan_key") => &["https://api.stepfun.com"],
        ("zai", "international:api_key") => &["https://api.z.ai"],
        ("zai", "china:api_key") => &["https://open.bigmodel.cn"],
        _ => &[],
    }
}
pub fn selected_origin(provider: &str) -> Result<&'static str, String> {
    let selection = current_selection(provider)?;
    remote_origins(provider, &selection)
        .first()
        .copied()
        .ok_or_else(configuration_required)
}
pub fn selected_local_origin(provider: &str) -> Result<String, String> {
    let selection = current_selection(provider)?;
    let origin = selection
        .strip_prefix("local:")
        .ok_or("Select an explicit local origin in Settings")?;
    local_origin(origin)
}

fn check_url(url: &Url) -> Result<(), String> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Embedded URL credentials are forbidden".into());
    }
    if url.fragment().is_some()
        || url.host_str().is_none()
        || !matches!(url.scheme(), "https" | "http")
    {
        return Err("Invalid network destination".into());
    }
    Ok(())
}
/// Origin-only input: no path, query, fragment or embedded user information.
pub fn canonical_origin(raw: &str) -> Result<String, String> {
    let url = Url::parse(raw).map_err(|_| "Invalid network origin")?;
    check_url(&url)?;
    if url.path() != "/" || url.query().is_some() {
        return Err("Configure an origin without a path or query".into());
    }
    Ok(url.origin().ascii_serialization())
}
pub fn local_origin(raw: &str) -> Result<String, String> {
    let canonical = canonical_origin(raw)?;
    let url = Url::parse(&canonical).map_err(|_| "Invalid local origin")?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
    if !local || url.port_or_known_default() == Some(0) {
        return Err("Only an explicit loopback origin and port are allowed".into());
    }
    Ok(canonical)
}
pub fn fixed_origin(raw: &str, expected: &str) -> Result<String, String> {
    let canonical = canonical_origin(raw)?;
    if canonical != expected {
        return Err("Unapproved service origin".into());
    }
    Ok(canonical)
}
pub fn validate_destination(endpoint: &ProviderEndpoint, url: &Url) -> Result<(), String> {
    check_url(url)?;
    let origin = canonical_origin(&endpoint.origin)?;
    if url.origin().ascii_serialization() != origin {
        return Err("Request origin differs from the selected endpoint".into());
    }
    if let Some(local) = endpoint.region.strip_prefix("local:") {
        if !matches!(endpoint.provider.as_str(), "ollama" | "antigravity")
            || local_origin(local)? != origin
        {
            return Err("Local destination was not explicitly bound".into());
        }
        return Ok(());
    }
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !remote_origins(&endpoint.provider, &endpoint.region).contains(&origin.as_str())
    {
        return Err("Destination is not approved for this provider and selected region".into());
    }
    if endpoint.provider == "commandcode"
        && (url.query().is_some()
            || !matches!(
                url.path(),
                "/alpha/billing/credits" | "/alpha/billing/subscriptions"
            ))
    {
        return Err("CommandCode permits only the two read-only billing endpoints".into());
    }
    Ok(())
}

pub const PUBLIC_PRICE_URLS: &[&str] = &[
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json",
    "https://models.dev/api.json",
    "https://robinebers.github.io/openusage/pricing_supplement.json",
];
pub fn validate_public_price_url(url: &Url) -> Result<(), String> {
    check_url(url)?;
    if PUBLIC_PRICE_URLS.contains(&url.as_str()) {
        Ok(())
    } else {
        Err("Unapproved public price source".into())
    }
}
/// Only listeners associated with the exact discovered process qualify.
pub(crate) fn antigravity_process_ports(raw: &str, pid: u32) -> Vec<u16> {
    let mut ports: Vec<u16> = raw
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() != 5
                || fields[0] != "TCP"
                || fields[3] != "LISTENING"
                || fields[4].parse::<u32>().ok() != Some(pid)
            {
                return None;
            }
            let (addr, port) = fields[1].rsplit_once(':')?;
            if !matches!(addr, "127.0.0.1" | "0.0.0.0") {
                return None;
            }
            port.parse::<u16>().ok().filter(|p| *p != 0)
        })
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}
