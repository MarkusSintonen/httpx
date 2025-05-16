use crate::exceptions::{BadMethodError, BadUrlError};
use pyo3::exceptions::PyRuntimeError;
use pyo3::{PyErr, PyResult};
use reqwest::{IntoUrl, Method, Url, Version};

pub fn parse_method(method: String) -> Result<Method, PyErr> {
    Method::from_bytes(method.as_bytes())
        .map_err(|e| BadMethodError::new_err(format!("Invalid HTTP method: {}", e)))
}

pub fn parse_url<U: IntoUrl>(url: U) -> Result<Url, PyErr> {
    url.into_url()
        .map_err(|e| BadUrlError::new_err(format!("Invalid URL: {}", e)))
}

pub fn http_version_str(version: Version) -> PyResult<String> {
    match version {
        Version::HTTP_09 => Ok("HTTP/0.9".to_string()),
        Version::HTTP_10 => Ok("HTTP/1.0".to_string()),
        Version::HTTP_11 => Ok("HTTP/1.1".to_string()),
        Version::HTTP_2 => Ok("HTTP/2".to_string()),
        Version::HTTP_3 => Ok("HTTP/3".to_string()),
        _ => Err(PyRuntimeError::new_err("Unknown HTTP version in response")),
    }
}
