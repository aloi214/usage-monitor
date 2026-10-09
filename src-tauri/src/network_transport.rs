//! Closed request surface: no raw builder/client access and no automatic redirects.
use crate::{
    access_policy::{self, AccessPermit},
    network_policy::{self, ProviderEndpoint},
};
use reqwest::{IntoUrl, Method, Response, Url};
use serde::Serialize;
use std::{fmt, time::Duration};

/// Do not retain reqwest's URL, body, headers, or source error in diagnostics.
#[derive(Clone)]
pub struct CheckedRequestError(String);
impl fmt::Display for CheckedRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl fmt::Debug for CheckedRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for CheckedRequestError {}
impl From<String> for CheckedRequestError {
    fn from(value: String) -> Self {
        Self(value)
    }
}
fn transport_error(_: reqwest::Error) -> CheckedRequestError {
    CheckedRequestError(
        "Network request failed (connection, TLS, timeout, or invalid request)".into(),
    )
}

pub struct CheckedClient {
    provider: String,
    proxy: Option<String>,
    process_origin: Option<String>,
}
struct AuthorizedRequest {
    client: reqwest::Client,
    builder: reqwest::RequestBuilder,
    permit: AccessPermit,
    endpoint: ProviderEndpoint,
    selection: String,
}
pub struct CheckedRequest {
    inner: Result<AuthorizedRequest, CheckedRequestError>,
}
impl CheckedClient {
    pub fn new(provider: &str, proxy: Option<&str>) -> Self {
        Self {
            provider: provider.into(),
            proxy: proxy.map(str::to_string),
            process_origin: None,
        }
    }
    /// Only Antigravity's process-discovery path may call this after matching
    /// the intended process and its listening port. Mode consent is checked
    /// again here and at send. A caller URL alone never enables local access.
    pub(crate) fn antigravity_local(origin: &str) -> Result<Self, String> {
        if network_policy::current_selection("antigravity")? != "local_process" {
            return Err("Enable Local Antigravity process mode in Settings".into());
        }
        let origin = network_policy::local_origin(origin)?;
        if Url::parse(&origin)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .as_deref()
            != Some("127.0.0.1")
        {
            return Err(
                "Antigravity process transport requires its discovered IPv4 loopback endpoint"
                    .into(),
            );
        }
        Ok(Self {
            provider: "antigravity".into(),
            proxy: None,
            process_origin: Some(origin),
        })
    }
    pub fn get(&self, url: impl IntoUrl) -> CheckedRequest {
        self.request(Method::GET, url)
    }
    pub fn post(&self, url: impl IntoUrl) -> CheckedRequest {
        self.request(Method::POST, url)
    }
    fn request(&self, method: Method, url: impl IntoUrl) -> CheckedRequest {
        let inner = (|| {
            access_policy::check_current_operation().map_err(CheckedRequestError::from)?;
            let permit = access_policy::current_operation()
                .ok_or_else(|| CheckedRequestError("Missing account operation".into()))?;
            let selection = network_policy::selection_for_permit(&self.provider, &permit)?;
            let url = url
                .into_url()
                .map_err(|_| CheckedRequestError("Invalid request URL".into()))?;
            let region =
                match (&self.process_origin, selection.as_str()) {
                    (Some(origin), "local_process") => format!("local:{origin}"),
                    (Some(_), _) | (None, "local_process") => return Err(CheckedRequestError(
                        "Local Antigravity endpoint must come from authorized process discovery"
                            .into(),
                    )),
                    _ => selection.clone(),
                };
            let endpoint = ProviderEndpoint {
                provider: self.provider.clone(),
                region,
                origin: url.origin().ascii_serialization(),
            };
            network_policy::validate_destination(&endpoint, &url)?;
            if self.provider == "commandcode" && method != Method::GET {
                return Err(CheckedRequestError(
                    "CommandCode billing is read-only".into(),
                ));
            }
            let local = endpoint.region.starts_with("local:");
            let mut builder = base_client();
            if local {
                builder = builder.no_proxy();
                // Never resolve a configurable hostname other than literal localhost.
                if url.host_str() == Some("localhost") {
                    builder = builder.resolve(
                        "localhost",
                        ([127, 0, 0, 1], url.port_or_known_default().unwrap_or(80)).into(),
                    );
                }
                if self.provider == "antigravity" {
                    builder = builder.danger_accept_invalid_certs(true);
                }
            } else if let Some(proxy) = &self.proxy {
                builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(transport_error)?);
            }
            #[cfg(test)]
            if let Ok((routes, cert)) = TEST_TRANSPORT.try_with(Clone::clone) {
                builder = builder.no_proxy().add_root_certificate(cert);
                for (host, addr) in routes {
                    builder = builder.resolve(&host, addr);
                }
            }
            let client = builder.build().map_err(transport_error)?;
            let builder = client.request(method, url);
            Ok(AuthorizedRequest {
                client,
                builder,
                permit,
                endpoint,
                selection,
            })
        })();
        CheckedRequest { inner }
    }
}
fn base_client() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("rice-monitor/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(5))
}
impl CheckedRequest {
    fn map(self, f: impl FnOnce(reqwest::RequestBuilder) -> reqwest::RequestBuilder) -> Self {
        Self {
            inner: self.inner.map(|mut request| {
                request.builder = f(request.builder);
                request
            }),
        }
    }
    pub fn header(self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        self.map(|b| b.header(key.as_ref(), value.as_ref()))
    }
    pub fn bearer_auth<T: fmt::Display>(self, value: T) -> Self {
        self.map(|b| b.bearer_auth(value))
    }
    pub fn json<T: Serialize + ?Sized>(self, value: &T) -> Self {
        self.map(|b| b.json(value))
    }
    pub fn form<T: Serialize + ?Sized>(self, value: &T) -> Self {
        self.map(|b| b.form(value))
    }
    pub fn body(self, value: impl Into<reqwest::Body>) -> Self {
        self.map(|b| b.body(value))
    }
    pub fn timeout(self, value: Duration) -> Self {
        self.map(|b| b.timeout(value))
    }
    pub async fn send(self) -> Result<Response, CheckedRequestError> {
        let authorized = self.inner?;
        access_policy::check_current_operation()?;
        authorized.permit.check()?;
        let current = access_policy::current_operation()
            .ok_or_else(|| CheckedRequestError("Missing account operation".into()))?;
        if current.account_id() != authorized.permit.account_id()
            || network_policy::selection_for_permit(&authorized.endpoint.provider, &current)?
                != authorized.selection
        {
            return Err(CheckedRequestError(
                "Account or network selection changed; retry explicitly".into(),
            ));
        }
        let request = authorized.builder.build().map_err(transport_error)?;
        network_policy::validate_destination(&authorized.endpoint, request.url())?;
        // Last synchronous gate before I/O. No await or callback can attach
        // credentials, change destinations, or pick a new account after it.
        authorized.permit.check()?;
        authorized
            .client
            .execute(request)
            .await
            .map_err(transport_error)
    }
}

/// A separate credential-free API. No headers/body/auth methods are exposed;
/// conditional ETags are the only caller-supplied metadata.
pub struct PublicPriceRequest {
    url: Result<Url, CheckedRequestError>,
    etag: Option<String>,
    proxy: Option<String>,
}
pub fn public_price_request(url: &str, proxy: Option<&str>) -> PublicPriceRequest {
    let url = Url::parse(url)
        .map_err(|_| CheckedRequestError("Invalid public price URL".into()))
        .and_then(|url| {
            network_policy::validate_public_price_url(&url)?;
            Ok(url)
        });
    PublicPriceRequest {
        url,
        etag: None,
        proxy: proxy.map(str::to_string),
    }
}
impl PublicPriceRequest {
    pub fn etag(mut self, etag: &str) -> Self {
        self.etag = Some(etag.into());
        self
    }
    pub async fn send(self) -> Result<Response, CheckedRequestError> {
        let url = self.url?;
        network_policy::validate_public_price_url(&url)?;
        let mut client = base_client();
        if let Some(proxy) = self.proxy {
            client = client.proxy(reqwest::Proxy::all(proxy).map_err(transport_error)?);
        }
        #[cfg(test)]
        if let Ok((routes, cert)) = TEST_TRANSPORT.try_with(Clone::clone) {
            client = client.no_proxy().add_root_certificate(cert);
            for (host, addr) in routes {
                client = client.resolve(&host, addr);
            }
        }
        let mut request = client.build().map_err(transport_error)?.get(url);
        if let Some(etag) = self.etag {
            request = request.header("If-None-Match", etag);
        }
        request.send().await.map_err(transport_error)
    }
}

// A test-only DNS/TLS fixture. It cannot alter destination validation, policy,
// selected region, method, headers, or redirect behavior. Release has no hook.
#[cfg(test)]
tokio::task_local! { static TEST_TRANSPORT: (Vec<(String, std::net::SocketAddr)>, reqwest::Certificate); }
#[cfg(test)]
pub(crate) async fn with_test_transport<T>(
    routes: Vec<(String, std::net::SocketAddr)>,
    cert: reqwest::Certificate,
    future: impl std::future::Future<Output = T>,
) -> T {
    TEST_TRANSPORT.scope((routes, cert), future).await
}
