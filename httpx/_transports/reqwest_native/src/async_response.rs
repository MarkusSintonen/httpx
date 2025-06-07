use crate::utils::{Extensions, HeaderMapExt, StatusCodeExt, VersionExt, map_read_error};
use pyo3::prelude::*;
use pyo3_bytes::PyBytes;
use std::collections::VecDeque;
use tokio::sync::OwnedSemaphorePermit;

#[pyclass]
pub struct Response {
    #[pyo3(get)]
    status_code: StatusCodeExt,
    #[pyo3(get)]
    headers: HeaderMapExt,
    #[pyo3(get)]
    http_version: VersionExt,
    #[pyo3(get)]
    extensions: Extensions,

    inner: Option<reqwest::Response>,
    request_semaphore_permit: Option<OwnedSemaphorePermit>,
    init_chunks: VecDeque<PyBytes>,
}

#[pymethods]
impl Response {
    async fn next_chunk<'py>(&mut self) -> PyResult<Option<PyBytes>> {
        if let Some(chunk) = self.init_chunks.pop_front() {
            return Ok(Some(chunk));
        }

        if let Some(inner) = self.inner.as_mut() {
            Ok(inner.chunk().await.map_err(map_read_error)?.map(PyBytes::new))
        } else {
            self.close();
            Ok(None)
        }
    }

    fn close(&mut self) {
        self.request_semaphore_permit.take().map(drop);
        self.inner.take().map(drop);
    }
}
impl Response {
    pub async fn initialize(
        mut response: reqwest::Response,
        mut request_semaphore_permit: Option<OwnedSemaphorePermit>,
    ) -> PyResult<Py<Response>> {
        let status_code = StatusCodeExt::from(response.status());
        let headers = HeaderMapExt(response.headers().clone());
        let http_version = VersionExt::from(response.version());
        let extensions = Extensions::from(response.extensions());

        let init_byte_limit = 65536;
        let (init_chunks, has_more) = Self::read_limit(&mut response, init_byte_limit).await?;

        let (resp, request_permit) = if has_more {
            (Some(response), request_semaphore_permit)
        } else {
            // Release the semaphore right away and drop the response
            // without waiting for user to do it (by consuming or closing).
            request_semaphore_permit.take().map(drop);
            drop(response);
            (None, None)
        };

        Python::with_gil(|py| {
            let response = Response {
                status_code,
                headers,
                http_version,
                extensions,
                inner: resp,
                request_semaphore_permit: request_permit,
                init_chunks,
            };
            Py::new(py, response)
        })
    }

    async fn read_limit(response: &mut reqwest::Response, byte_limit: usize) -> PyResult<(VecDeque<PyBytes>, bool)> {
        let mut init_chunks: VecDeque<PyBytes> = VecDeque::new();
        let mut has_more = true;
        let mut tot_bytes = 0;
        while has_more && (tot_bytes < byte_limit) {
            if let Some(chunk) = response.chunk().await.map_err(map_read_error)? {
                if !chunk.is_empty() {
                    tot_bytes += chunk.len();
                    init_chunks.push_back(PyBytes::new(chunk));
                }
            } else {
                has_more = false;
            }
        }
        Ok((init_chunks, has_more))
    }
}
impl Drop for Response {
    fn drop(&mut self) {
        self.close()
    }
}
