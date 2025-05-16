use crate::exceptions::{BadHeaderError, BadUrlError};
use crate::utils::parse_url;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use reqwest::Proxy;
use reqwest::header::{HeaderName, HeaderValue};

#[pyclass]
#[derive(Clone)]
pub struct NativeProxyConfig {
    #[pyo3(get)]
    url: String,
    #[pyo3(get)]
    basic_auth: Option<(String, String)>,
    #[pyo3(get)]
    headers: Option<Vec<(Vec<u8>, Vec<u8>)>>,
}

#[pymethods]
impl NativeProxyConfig {
    #[new]
    fn py_new(
        url: String,
        basic_auth: Option<(String, String)>,
        headers: Option<Vec<(Vec<u8>, Vec<u8>)>>,
    ) -> PyResult<Self> {
        Ok(NativeProxyConfig {
            url,
            basic_auth,
            headers,
        })
    }
}

impl NativeProxyConfig {
    pub fn build_reqwest_proxy(&self) -> PyResult<Proxy> {
        let url = parse_url(&self.url)?;
        if url.scheme() != "http"
            && url.scheme() != "https"
            && url.scheme() != "socks5"
            && url.scheme() != "socks5h"
        {
            return Err(BadUrlError::new_err(format!(
                "Invalid URL scheme: {}",
                url.scheme()
            )));
        }

        let mut proxy = Proxy::all(url)
            .map_err(|e| PyValueError::new_err(format!("Invalid Proxy URL: {}", e)))?;

        if let Some((username, password)) = &self.basic_auth {
            proxy = proxy.basic_auth(username, password);
        }
        if let Some(headers) = &self.headers {
            // Check there is only Proxy-Authorization header
            // https://github.com/seanmonstar/reqwest/issues/2552
            if headers.len() > 1 {
                return Err(BadHeaderError::new_err(
                    "Only Proxy-Authorization header is allowed, for now.",
                ));
            }
            if let Some((name, value)) = headers.first() {
                let header_name = HeaderName::from_bytes(name)
                    .map_err(|_| BadHeaderError::new_err("Invalid header name"))?;
                if header_name.as_str().to_lowercase() != "proxy-authorization" {
                    return Err(BadHeaderError::new_err(
                        "Only Proxy-Authorization header is allowed, for now.",
                    ));
                }
                let custom_http_auth = HeaderValue::from_bytes(value)
                    .map_err(|_| BadHeaderError::new_err("Invalid header value"))?;
                proxy = proxy.custom_http_auth(custom_http_auth);
            }
        }

        Ok(proxy)
    }
}
