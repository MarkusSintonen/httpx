use crate::exceptions::{ReadConnectionError, ReadTimeoutError, ReadUnknownError};
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use reqwest::{Response, Version};
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedSemaphorePermit};

#[pyclass]
pub struct NativeAsyncResponse {
    #[pyo3(get)]
    status: u16,
    #[pyo3(get)]
    headers: Vec<(Vec<u8>, Vec<u8>)>,
    #[pyo3(get)]
    http_version: String,
    response: Option<Arc<Mutex<Response>>>,
    request_semaphore_permit: Option<OwnedSemaphorePermit>,
}

impl Drop for NativeAsyncResponse {
    fn drop(&mut self) {
        if let Some(request_semaphore_permit) = self.request_semaphore_permit.take() {
            drop(request_semaphore_permit);
        }
    }
}

impl NativeAsyncResponse {
    pub fn new(
        response: Response,
        request_semaphore_permit: Option<OwnedSemaphorePermit>,
    ) -> PyResult<Self> {
        let response = NativeAsyncResponse {
            status: response.status().as_u16(),
            headers: response
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str().as_bytes().to_vec(), v.as_bytes().to_vec()))
                .collect(),
            http_version: Self::http_version_str(response.version())?,
            response: Some(Arc::new(Mutex::new(response))),
            request_semaphore_permit,
        };
        Ok(response)
    }
}

#[pymethods]
impl NativeAsyncResponse {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let response = self
            .response
            .clone()
            .ok_or_else(|| PyRuntimeError::new_err("Response is not initialized"))?;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            match response.lock().await.chunk().await {
                Ok(Some(chunk)) => Ok(Python::with_gil(|py| {
                    PyBytes::new(py, chunk.as_ref()).unbind()
                })),
                Ok(None) => Err(PyStopAsyncIteration::new_err("End of stream")),
                Err(e) => Err(Self::map_read_error(e)),
            }
        })
    }

    fn close<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        if let Some(request_semaphore_permit) = self.request_semaphore_permit.take() {
            drop(request_semaphore_permit);
        }

        let mut response = self.response.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            response.take(); // Drop the response
            Ok(())
        })
    }
}

impl NativeAsyncResponse {
    fn http_version_str(version: Version) -> PyResult<String> {
        match version {
            Version::HTTP_09 => Ok("HTTP/0.9".to_string()),
            Version::HTTP_10 => Ok("HTTP/1.0".to_string()),
            Version::HTTP_11 => Ok("HTTP/1.1".to_string()),
            Version::HTTP_2 => Ok("HTTP/2".to_string()),
            Version::HTTP_3 => Ok("HTTP/3".to_string()),
            _ => Err(PyRuntimeError::new_err("Unknown HTTP version in response")),
        }
    }

    fn map_read_error(error: reqwest::Error) -> PyErr {
        if error.is_connect() {
            ReadConnectionError::new_err(format!("Connection error on read: {}", error))
        } else if error.is_timeout() {
            ReadTimeoutError::new_err(format!("Timeout on read: {}", error))
        } else {
            ReadUnknownError::new_err(format!("Unknown failure on read: {}", error))
        }
    }
}
