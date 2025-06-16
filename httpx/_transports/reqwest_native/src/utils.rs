use crate::exceptions::{
    ReadConnectionError, ReadError, ReadTimeoutError, SendConnectionError, SendError, SendTimeoutError,
};
use crate::http_types::Extensions;
use pyo3::PyErr;
use std::error::Error;

pub fn copy_extensions<'a>(from: &Extensions, to: &'a mut http::Extensions) -> &'a mut Extensions {
    let to = to.get_or_insert_default::<Extensions>();
    for (k, v) in from.0.iter() {
        if !to.0.contains_key(k) {
            to.0.insert(k.clone(), v.clone());
        }
    }
    to
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
