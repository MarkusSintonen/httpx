use crate::exceptions::{BadMethodError, BadUrlError};
use bytes::Bytes;
use http::HeaderMap;
use pyo3::exceptions::PyNotImplementedError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use pyo3::{Bound, FromPyObject, IntoPyObject, PyErr, Python};
use reqwest::{IntoUrl, Method, Url, Version};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;

pub fn parse_method(method: String) -> Result<Method, PyErr> {
    Method::from_bytes(method.as_bytes())
        .map_err(|e| BadMethodError::new_err(format!("Invalid HTTP method: {}", e)))
}

pub fn parse_url<U: IntoUrl>(url: U) -> Result<Url, PyErr> {
    url.into_url()
        .map_err(|e| BadUrlError::new_err(format!("Invalid URL: {}", e)))
}

pub fn http_version_str(version: Version) -> String {
    format!("{:?}", version)
}

pub fn headers_to_bytes(headers: &HeaderMap) -> Vec<(Vec<u8>, Vec<u8>)> {
    headers
        .iter()
        .map(|(k, v)| (k.as_str().as_bytes().to_vec(), v.as_bytes().to_vec()))
        .collect()
}

#[derive(Debug)]
pub struct NotImplementedError {
    pub message: String,
}
impl fmt::Display for NotImplementedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl Error for NotImplementedError {}
impl From<NotImplementedError> for PyErr {
    fn from(err: NotImplementedError) -> PyErr {
        PyNotImplementedError::new_err(err.message)
    }
}

#[derive(Clone, Debug)]
pub struct BytesExt(Bytes);
impl From<Vec<u8>> for BytesExt {
    fn from(bytes: Vec<u8>) -> Self {
        BytesExt(Bytes::from(bytes))
    }
}
impl<'py> IntoPyObject<'py> for BytesExt {
    type Target = PyBytes;
    type Output = Bound<'py, PyBytes>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok(PyBytes::new(py, &self.0))
    }
}
impl<'py> FromPyObject<'py> for BytesExt {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let slice: &[u8] = ob.extract()?;
        Ok(BytesExt(Bytes::copy_from_slice(slice)))
    }
}

#[derive(FromPyObject, IntoPyObject, Clone, Debug)]
pub enum ExtensionPrimitive {
    Str(String),
    I64(i64),
    U64(u64),
    F64(f64),
    Bool(bool),
    Bytes(BytesExt),
}
#[derive(FromPyObject, IntoPyObject, Clone, Debug)]
pub enum ExtensionValue {
    Str(String),
    I64(i64),
    U64(u64),
    F64(f64),
    Bool(bool),
    Bytes(BytesExt),
    Dict(HashMap<String, ExtensionPrimitive>),
    List(Vec<ExtensionPrimitive>),
}
pub type Extensions = HashMap<String, ExtensionValue>;

pub fn extensions_to_dict<'py>(
    py: Python<'py>,
    extensions: &Extensions,
) -> Result<Bound<'py, PyDict>, PyErr> {
    let dict = PyDict::new(py);
    for (key, value) in extensions.iter() {
        dict.set_item(key, value.clone())?;
    }
    Ok(dict)
}

pub fn http_extensions_to_dict<'py>(
    py: Python<'py>,
    http_extensions: &http::Extensions,
) -> Result<Bound<'py, PyDict>, PyErr> {
    match http_extensions.get::<Extensions>() {
        Some(ext) => extensions_to_dict(py, ext),
        None => Ok(PyDict::new(py)),
    }
}

pub fn extensions_from_dict(extensions: &Bound<PyDict>) -> Result<Extensions, PyErr> {
    let mut ext = Extensions::new();
    for (key, value) in extensions.iter() {
        let key: String = key.extract()?;
        let value: ExtensionValue = value.extract()?;
        ext.insert(key, value);
    }
    Ok(ext)
}
