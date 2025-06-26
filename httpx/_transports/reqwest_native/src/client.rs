use crate::http_types::{MethodExt, UrlExt};
use crate::request_builder::RequestBuilder;
use crate::runtime::Runtime;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

#[pyclass]
pub struct Client {
    client: Option<Arc<reqwest::Client>>,
    runtime: Arc<Runtime>,
    middlewares: Option<Arc<Vec<Py<PyAny>>>>,
    request_semaphore: Option<Arc<Semaphore>>,
    connect_timeout: Option<Duration>,
}

#[pymethods]
impl Client {
    fn request(&self, method: MethodExt, url: UrlExt) -> PyResult<RequestBuilder> {
        let runtime = self.runtime.clone();
        let client = self
            .client
            .clone()
            .ok_or_else(|| PyRuntimeError::new_err("Client is not initialized"))?;

        let request_semaphore = self.request_semaphore.clone();
        let connect_timeout = self.connect_timeout.clone();
        let middlewares = self.middlewares.clone();

        let url: reqwest::Url = url.try_into()?;
        let inner = client.request(method.0, url);
        Ok(RequestBuilder::new(runtime, inner, middlewares, request_semaphore, connect_timeout))
    }

    async fn close(&self) {
        self.runtime.close().await;
    }
}

impl Client {
    pub fn new(
        inner: reqwest::Client,
        runtime: Runtime,
        middlewares: Option<Vec<Py<PyAny>>>,
        max_connections: Option<usize>,
        connect_timeout: Option<Duration>,
    ) -> Self {
        Client {
            client: Some(Arc::new(inner)),
            runtime: Arc::new(runtime),
            request_semaphore: max_connections.map(|limit| Arc::new(Semaphore::new(limit))),
            middlewares: middlewares.map(Arc::new),
            connect_timeout,
        }
    }
}
