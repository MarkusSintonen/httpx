use crate::exceptions::{
    ReadConnectionError, ReadError, ReadTimeoutError, SendConnectionError, SendError, SendTimeoutError,
};
use http::HeaderMap;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::PyType;
use pyo3::{Bound, FromPyObject, IntoPyObject, PyErr, Python};
use pyo3_bytes::PyBytes;
use pythonize::{depythonize, pythonize};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::Debug;

pub fn copy_extensions<'a>(from: &Extensions, to: &'a mut http::Extensions) -> &'a mut Extensions {
    let to = to.get_or_insert_default::<Extensions>();
    for (k, v) in from.0.iter() {
        if !to.0.contains_key(k) {
            to.0.insert(k.clone(), v.clone());
        }
    }
    to
}

#[derive(Serialize, Deserialize, Clone)]
pub struct UrlExt(#[serde(with = "http_serde::uri")] pub http::Uri);
#[derive(Serialize, Deserialize, Clone)]
pub struct MethodExt(#[serde(with = "http_serde::method")] pub http::Method);
#[derive(Serialize, Deserialize, Clone)]
pub struct HeaderMapExt(#[serde(with = "http_serde::header_map")] pub HeaderMap);
#[derive(Serialize, Deserialize, Clone)]
pub struct VersionExt(#[serde(with = "http_serde::version")] pub http::Version);
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Extensions(pub serde_json::Map<String, serde_json::Value>);
#[derive(Serialize, Deserialize, Clone)]
pub struct StatusCodeExt(#[serde(with = "http_serde::status_code")] pub http::StatusCode);

impl<'py> IntoPyObject<'py> for UrlExt {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;
    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok(pythonize(py, &self)?)
    }
}
impl<'py> FromPyObject<'py> for UrlExt {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(depythonize(ob)?)
    }
}
impl TryFrom<reqwest::Url> for UrlExt {
    type Error = PyErr;
    fn try_from(value: reqwest::Url) -> PyResult<Self> {
        Ok(UrlExt(
            value
                .as_str()
                .parse()
                .map_err(|e| PyValueError::new_err(format!("Invalid URL format: {}", e)))?,
        ))
    }
}
impl TryInto<reqwest::Url> for UrlExt {
    type Error = PyErr;
    fn try_into(self) -> PyResult<reqwest::Url> {
        self.0
            .to_string()
            .parse::<reqwest::Url>()
            .map_err(|e| PyValueError::new_err(format!("Invalid URL format: {}", e)))
    }
}

impl<'py> IntoPyObject<'py> for MethodExt {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;
    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok(pythonize(py, &self)?)
    }
}
impl<'py> FromPyObject<'py> for MethodExt {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(depythonize(ob)?)
    }
}
impl From<reqwest::Method> for MethodExt {
    fn from(method: reqwest::Method) -> Self {
        MethodExt(method)
    }
}

impl<'py> IntoPyObject<'py> for VersionExt {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;
    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok(pythonize(py, &self)?)
    }
}
impl<'py> FromPyObject<'py> for VersionExt {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(depythonize(ob)?)
    }
}
impl From<reqwest::Version> for VersionExt {
    fn from(version: reqwest::Version) -> Self {
        VersionExt(version)
    }
}

impl<'py> IntoPyObject<'py> for HeaderMapExt {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;
    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        let dict = multidict(py)?;
        for (key, value) in self.0.iter() {
            dict.set_item(key.as_str(), value.as_bytes())?;
        }
        Ok(dict)
    }
}
impl<'py> FromPyObject<'py> for HeaderMapExt {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(depythonize(ob)?)
    }
}
impl From<HeaderMap> for HeaderMapExt {
    fn from(header_map: HeaderMap) -> Self {
        HeaderMapExt(header_map)
    }
}

impl<'py> IntoPyObject<'py> for Extensions {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;
    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok(pythonize(py, &self)?)
    }
}
impl<'py> FromPyObject<'py> for Extensions {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(depythonize(ob)?)
    }
}
impl From<&http::Extensions> for Extensions {
    fn from(http_extensions: &http::Extensions) -> Self {
        match http_extensions.get::<Extensions>() {
            Some(ext) => Extensions(ext.0.clone()),
            None => Extensions(serde_json::Map::new()),
        }
    }
}

impl<'py> IntoPyObject<'py> for StatusCodeExt {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;
    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok(pythonize(py, &self)?)
    }
}
impl<'py> FromPyObject<'py> for StatusCodeExt {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(depythonize(ob)?)
    }
}
impl From<http::StatusCode> for StatusCodeExt {
    fn from(status: http::StatusCode) -> Self {
        StatusCodeExt(status)
    }
}

fn multidict(py: Python) -> PyResult<Bound<PyAny>> {
    static MULTIDICT_CELL: GILOnceCell<Py<PyType>> = GILOnceCell::new();
    MULTIDICT_CELL.import(py, "multidict", "CIMultiDict")?.call0()
}

pub fn map_send_error(error: reqwest::Error) -> PyErr {
    if error.is_connect() {
        SendConnectionError::new_err(format!("Connection error on send: {}", error))
    } else if error.is_timeout() {
        SendTimeoutError::new_err(format!("Timeout on send: {}", error))
    } else {
        SendError::new_err(format!("Unknown failure on send: {:?}", error.source()))
    }
}

pub fn map_read_error(error: reqwest::Error) -> PyErr {
    if error.is_connect() {
        ReadConnectionError::new_err(format!("Connection error on read: {}", error))
    } else if error.is_timeout() {
        ReadTimeoutError::new_err(format!("Timeout on read: {}", error))
    } else {
        ReadError::new_err(format!("Unknown failure on read: {}", error))
    }
}

#[derive(FromPyObject, IntoPyObject)]
pub enum Body {
    Str(String),
    Bytes(PyBytes),
}
