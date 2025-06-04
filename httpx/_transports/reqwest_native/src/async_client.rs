use crate::async_response::Response;
use crate::asyncio::{py_async_gen_to_stream};
use crate::exceptions::PoolTimeoutError;
use crate::middleware::MiddlewareAdapter;
use crate::proxy_config::NativeProxyConfig;
use crate::runtime::Runtime;
use crate::utils::{Extensions, HeaderMapExt, MethodExt, UrlExt, copy_extensions, map_send_error};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3_bytes::PyBytes;
use reqwest::Client;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[pyclass]
pub struct NativeAsyncClient {
    client: Option<ClientWithMiddleware>,
    request_semaphore: Option<Arc<Semaphore>>,
    connect_timeout: Option<Duration>,
    #[pyo3(get)]
    proxy: Option<NativeProxyConfig>,
    runtime: Runtime,
}

impl Drop for NativeAsyncClient {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            drop(client); // Explicitly drop the client
        }
    }
}

#[pymethods]
impl NativeAsyncClient {
    #[new]
    fn py_new(
        py: Python,
        total_timeout: Option<Duration>,
        connect_timeout: Option<Duration>,
        read_timeout: Option<Duration>,
        pool_idle_timeout: Option<Duration>,
        pool_max_idle_per_host: Option<usize>,
        max_connections: Option<usize>,
        http1: bool,
        http2: bool,
        root_certificates_der: Option<Vec<Vec<u8>>>,
        proxy: Option<NativeProxyConfig>,
        middlewares: Option<Vec<Bound<PyAny>>>,
    ) -> PyResult<Self> {
        if !http1 && !http2 {
            return Err(PyValueError::new_err("At least one of http1 or http2 must be true"));
        }
        if let Some(max_conns) = max_connections {
            if max_conns == 0 {
                return Err(PyValueError::new_err("max_connections must be greater than 0"));
            }
        }

        let mut client = Client::builder();
        if !http2 {
            client = client.http1_only();
        }
        if !http1 {
            client = client.http2_prior_knowledge();
        }
        if let Some(total_timeout) = total_timeout {
            client = client.timeout(total_timeout);
        }
        if let Some(connect_timeout) = connect_timeout {
            client = client.connect_timeout(connect_timeout);
        }
        if let Some(read_timeout) = read_timeout {
            client = client.read_timeout(read_timeout);
        }
        if let Some(pool_idle_timeout) = pool_idle_timeout {
            client = client.pool_idle_timeout(pool_idle_timeout);
        }
        if let Some(pool_max_idle_per_host) = pool_max_idle_per_host {
            client = client.pool_max_idle_per_host(pool_max_idle_per_host);
        }
        if let Some(root_certificates_der) = root_certificates_der {
            for cert in root_certificates_der {
                client = client.add_root_certificate(
                    reqwest::Certificate::from_der(&cert)
                        .map_err(|e| PyValueError::new_err(format!("Invalid certificate: {}", e)))?,
                );
            }
        }
        if let Some(proxy) = &proxy {
            client = client.proxy(proxy.build_reqwest_proxy()?);
        }

        let client = client
            .build()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create HTTP client: {}", e)))?;
        let mut middleware_client = ClientBuilder::new(client);

        if let Some(middlewares) = middlewares {
            for middleware in middlewares {
                middleware_client = middleware_client.with(MiddlewareAdapter::new(py, middleware)?);
            }
        }

        Ok(NativeAsyncClient {
            client: Some(middleware_client.build()),
            request_semaphore: max_connections.map(|limit| Arc::new(Semaphore::new(limit))),
            connect_timeout,
            proxy,
            runtime: Runtime::start()?,
        })
    }

    async fn request(
        &self,
        method: MethodExt,
        url: UrlExt,
        headers: Option<HeaderMapExt>,
        body: Option<Body>,
        stream: Option<PyObject>,
        timeout: Option<Duration>,
        extensions: Option<Extensions>,
    ) -> PyResult<Py<Response>> {
        let client = self
            .client
            .clone()
            .ok_or_else(|| PyRuntimeError::new_err("Client is not initialized"))?;

        let url: reqwest::Url = url.try_into()?;

        let mut body = match body {
            Some(Body::Str(body)) => Some(reqwest::Body::from(body)),
            Some(Body::Bytes(body)) => Some(reqwest::Body::from(body.into_inner())),
            None => None,
        };
        if let Some(stream) = stream {
            body = Some(reqwest::Body::wrap_stream(py_async_gen_to_stream(stream)));
        };

        let request_semaphore = self.request_semaphore.clone();
        let connect_timeout = self.connect_timeout.clone();

        let mut req_builder = client.request(method.0, url);
        if let Some(body) = body {
            req_builder = req_builder.body(body);
        }
        if let Some(headers) = headers {
            req_builder = req_builder.headers(headers.0);
        }
        if let Some(timeout) = timeout {
            req_builder = req_builder.timeout(timeout);
        }
        let extensions2 = extensions.clone();
        if let Some(extensions) = extensions {
            req_builder = req_builder.with_extension(extensions);
        }

        self.runtime
            .spawn(async move {
                let permit = if let Some(request_semaphore) = request_semaphore {
                    Some(Self::limit_connections(request_semaphore, connect_timeout).await?)
                } else {
                    None
                };

                let mut response = req_builder.send().await.map_err(map_send_error)?;

                if let Some(extensions) = extensions2 {
                    copy_extensions(&extensions, response.extensions_mut());
                }

                Response::initialize(response, permit).await
            })?
            .await
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to execute request: {}", e)))?
    }

    fn close(&mut self) {
        self.client.take().map(drop);
    }
}

impl NativeAsyncClient {
    async fn limit_connections(
        request_semaphore: Arc<Semaphore>,
        connect_timeout: Option<Duration>,
    ) -> PyResult<OwnedSemaphorePermit> {
        let permit = if let Some(connect_timeout) = connect_timeout {
            tokio::time::timeout(connect_timeout, request_semaphore.acquire_owned())
                .await
                .map_err(|_| PoolTimeoutError::new_err("Timeout acquiring semaphore"))?
        } else {
            request_semaphore.acquire_owned().await
        };
        permit.map_err(|e| PyRuntimeError::new_err(format!("Failed to acquire semaphore: {}", e)))
    }
}

#[derive(FromPyObject, IntoPyObject)]
pub enum Body {
    Str(String),
    Bytes(PyBytes),
}
