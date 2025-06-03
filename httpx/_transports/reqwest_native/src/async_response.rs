use std::collections::VecDeque;
use crate::asyncio::{future_to_coro, resolve_coro};
use crate::runtime::Runtime;
use crate::utils::{Extensions, HeaderMapExt, StatusCodeExt, VersionExt, map_read_error};
use bytes::Bytes;
use pyo3::IntoPyObjectExt;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};
use pyo3::prelude::*;
use pyo3_bytes::PyBytes;
use std::sync::Arc;
use pyo3::types::PyNone;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::OwnedSemaphorePermit;

#[pyclass]
pub struct HeadResponse {
    #[pyo3(get)]
    status_code: StatusCodeExt,
    #[pyo3(get)]
    headers: HeaderMapExt,
    #[pyo3(get)]
    http_version: VersionExt,
    #[pyo3(get)]
    extensions: Extensions,
}

#[pyclass]
pub struct BodyResponse {
    #[pyo3(get)]
    head: Py<HeadResponse>,
    #[pyo3(get)]
    body: Py<PyAny>,
}

#[pyclass]
pub struct StreamResponse {
    #[pyo3(get)]
    head: Py<HeadResponse>,
    stream: Option<Py<StreamReader>>,
}

#[pyclass]
struct StreamReader {
    response: Option<Arc<tokio::sync::Mutex<ResponseExt>>>,
    rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<PyResult<Vec<PyBytes>>>>>,
    runtime: Arc<Runtime>,
}

impl BodyResponse {
    pub fn new_py(head: HeadResponse, body: Vec<PyBytes>) -> PyResult<Py<PyAny>> {
        Python::with_gil(|py| {
            let response = BodyResponse {
                head: Py::new(py, head)?,
                body: body.into_py_any(py)?,
            };
            response.into_py_any(py)
        })
    }
}
impl StreamResponse {
    pub fn new_py(response: ResponseExt, head: HeadResponse, runtime: Arc<Runtime>) -> PyResult<Py<PyAny>> {
        Python::with_gil(|py| {
            let (tx, rx) = tokio::sync::mpsc::channel::<PyResult<Vec<PyBytes>>>(10);
            let stream = StreamReader {
                response: Some(Arc::new(tokio::sync::Mutex::new(response))),
                rx: Arc::new(tokio::sync::Mutex::new(rx)),
                runtime,
            };
            stream.start(tx)?;

            let response = StreamResponse {
                head: Py::new(py, head)?,
                stream: Some(Py::new(py, stream)?),
            };
            response.into_py_any(py)
        })
    }
}

#[pymethods]
impl StreamResponse {
    #[getter]
    fn body<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        if let Some(body) = &self.stream {
            Ok(body.into_bound_py_any(py)?)
        } else {
            Err(PyRuntimeError::new_err("Stream already closed"))
        }
    }

    fn close(&mut self) {
        if let Some(body) = self.stream.take() {
            drop(body);
        }
    }
}

#[pymethods]
impl StreamReader {
    fn wait_next<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match self.try_next_no_wait() {
            Ok((None, true)) => {} // Continue to wait for the next chunk
            Err(e) => return Err(e),
            Ok(res) => return resolve_coro(py, res),
        }

        let rx = self.rx.clone();

        future_to_coro(py, &self.runtime, async move {
            match rx.lock().await.recv().await {
                Some(chunks) => Ok((Some(chunks?), true)),
                None => Ok((None, false)),
            }
        })
    }

    fn try_next_no_wait(&mut self) -> PyResult<(Option<Vec<PyBytes>>, bool)> {
        if let Ok(mut rx) = self.rx.try_lock() {
            return match rx.try_recv() {
                Ok(chunks) => Ok((Some(chunks?), true)),              // Data available immediately
                Err(TryRecvError::Empty) => Ok((None, true)),         // No data available yet, but more coming
                Err(TryRecvError::Disconnected) => Ok((None, false)), // All data has been read
            };
        }
        Ok((None, true)) // No data available yet, but more coming
    }
}
impl StreamReader {
    fn start(&self, tx: tokio::sync::mpsc::Sender<PyResult<Vec<PyBytes>>>) -> PyResult<()> {
        let response = self
            .response
            .clone()
            .ok_or_else(|| PyRuntimeError::new_err("Response is not initialized"))?;

        self.runtime.spawn(async move {
            loop {
                match response.lock().await.read_limit().await {
                    Ok((chunks, has_more)) => {
                        if !chunks.is_empty() {
                            if tx.send(Ok(chunks)).await.is_err() {
                                return; // Channel closed, exit the loop
                            }
                        }
                        if !has_more {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        return; // Error occurred, exit the loop
                    }
                }
            }
        })?;
        Ok(())
    }
}

pub struct ResponseExt {
    inner: Option<reqwest::Response>,
    request_semaphore_permit: Option<OwnedSemaphorePermit>,
    init_chunks: Option<VecDeque<PyBytes>>,
    next_chunk: Option<PyBytes>,
}
impl ResponseExt {
    pub fn new(resp: reqwest::Response, request_semaphore_permit: Option<OwnedSemaphorePermit>) -> Self {
        ResponseExt {
            inner: Some(resp),
            request_semaphore_permit,
            init_chunks: None,
            next_chunk: None,
        }
    }

    pub async fn read_limit(&mut self) -> PyResult<(Vec<PyBytes>, bool)> {
        let byte_limit = 65536;
        let mut tot_bytes = 0;
        let mut chunks: Vec<PyBytes> = Vec::new();
        let mut stop = false;
        while let Some(chunk) = self.read_one().await? {
            let b: &Bytes = chunk.as_ref();
            if b.is_empty() {
                continue; // Make sure no empty chunks are added
            }
            tot_bytes += b.len();
            chunks.push(chunk);
            if stop {
                break
            }
            if tot_bytes >= byte_limit {
                stop = true // Read once more to see if there are more chunks
            }
        }

        if tot_bytes <= byte_limit {
            self.next_chunk = None;  // No more data to read
            return Ok((chunks, false));
        }

        if chunks.len() == 1 {
            let mut chunk = chunks.pop().unwrap().into_inner();
            self.next_chunk = Some(PyBytes::new(chunk.split_off(byte_limit)));
            chunks.push(PyBytes::new(chunk));
            return Ok((chunks, true));
        }

        self.next_chunk = chunks.pop();
        Ok((chunks, true))
    }

    pub fn set_init_chunks(&mut self, chunks: Vec<PyBytes>) {
        assert!(self.init_chunks.is_none());
        self.init_chunks = Some(VecDeque::from(chunks));
    }

    async fn read_one(&mut self) -> PyResult<Option<PyBytes>> {
        if let Some(ref mut chunks) = self.init_chunks {
            if let Some(chunk) = chunks.pop_front() {
                return Ok(Some(chunk));
            }
        }
        if let Some(chunk) = self.next_chunk.take() {
            return Ok(Some(chunk));
        }

        let Some(response) = self.inner.as_mut() else {
            return Ok(None); // Response has already been consumed
        };

        let chunk = response.chunk().await.map_err(map_read_error)?;
        if chunk.is_none() {
            // No more data is available, release the semaphore permit right away
            let _ = self.request_semaphore_permit.take();
            let _ = self.inner.take(); // Drop the response
        }
        Ok(chunk.map(PyBytes::new))
    }
}

impl From<&reqwest::Response> for HeadResponse {
    fn from(resp: &reqwest::Response) -> Self {
        HeadResponse {
            status_code: resp.status().into(),
            headers: HeaderMapExt(resp.headers().clone()),
            http_version: resp.version().into(),
            extensions: resp.extensions().into(),
        }
    }
}
