use serde::{Deserialize, Serialize};

/// A browser cookie — does not expose any CDP types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: f64,
    pub http_only: bool,
    pub secure: bool,
    #[serde(default)]
    pub same_site: Option<String>,
}

impl From<chromiumoxide::cdp::browser_protocol::network::Cookie> for Cookie {
    fn from(c: chromiumoxide::cdp::browser_protocol::network::Cookie) -> Self {
        Self {
            name: c.name,
            value: c.value,
            domain: c.domain,
            path: c.path,
            expires: c.expires,
            http_only: c.http_only,
            secure: c.secure,
            same_site: c.same_site.map(|s| format!("{s:?}")),
        }
    }
}

/// Parameters for setting a cookie.
#[derive(Debug, Clone)]
pub struct SetCookieParams {
    pub name: String,
    pub value: String,
    pub url: Option<String>,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub secure: Option<bool>,
    pub http_only: Option<bool>,
    pub same_site: Option<String>,
    pub expires: Option<f64>,
}

impl SetCookieParams {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            url: None,
            domain: None,
            path: None,
            secure: None,
            http_only: None,
            same_site: None,
            expires: None,
        }
    }

    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

impl From<SetCookieParams> for chromiumoxide::cdp::browser_protocol::network::CookieParam {
    fn from(p: SetCookieParams) -> Self {
        let mut cp =
            chromiumoxide::cdp::browser_protocol::network::CookieParam::new(p.name, p.value);
        cp.url = p.url;
        cp.domain = p.domain;
        cp.path = p.path;
        cp.secure = p.secure;
        cp.http_only = p.http_only;
        cp.expires = p
            .expires
            .map(|e| chromiumoxide::cdp::browser_protocol::network::TimeSinceEpoch::new(e));
        cp
    }
}

/// Parameters for deleting cookies.
#[derive(Debug, Clone)]
pub struct DeleteCookieParams {
    pub name: String,
    pub url: Option<String>,
    pub domain: Option<String>,
    pub path: Option<String>,
}

impl DeleteCookieParams {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: None,
            domain: None,
            path: None,
        }
    }
}

impl From<DeleteCookieParams>
    for chromiumoxide::cdp::browser_protocol::network::DeleteCookiesParams
{
    fn from(p: DeleteCookieParams) -> Self {
        let mut dp =
            chromiumoxide::cdp::browser_protocol::network::DeleteCookiesParams::new(p.name);
        dp.url = p.url;
        dp.domain = p.domain;
        dp.path = p.path;
        dp
    }
}
