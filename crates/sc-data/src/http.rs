//! Minimal HTTP abstraction so the fetch logic can be tested without network.

use std::time::Duration;

use crate::DataError;

/// Blocking HTTP GET returning the body of a 2xx response.
pub trait HttpClient: Send + Sync {
    fn get(&self, url: &str, accept: Option<&str>) -> Result<Vec<u8>, DataError>;
}

/// Largest body we accept (scunpacked files are a few MB).
const MAX_BODY: u64 = 64 * 1024 * 1024;

/// [`HttpClient`] backed by `ureq`, using the OS trust store and `HTTPS_PROXY` if set.
#[derive(Debug, Clone)]
pub struct UreqClient {
    agent: ureq::Agent,
}

impl UreqClient {
    pub fn new() -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("sc-companion/", env!("CARGO_PKG_VERSION")))
            .tls_config(tls)
            .build()
            .into();
        Self { agent }
    }
}

impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient for UreqClient {
    fn get(&self, url: &str, accept: Option<&str>) -> Result<Vec<u8>, DataError> {
        let mut req = self.agent.get(url);
        if let Some(accept) = accept {
            req = req.header("Accept", accept);
        }
        let err = |e: ureq::Error| DataError::Http {
            url: url.to_owned(),
            message: e.to_string(),
        };
        let mut resp = req.call().map_err(err)?;
        resp.body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_vec()
            .map_err(err)
    }
}
