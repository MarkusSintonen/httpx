use crate::async_response::Response;
use crate::asyncio::py_async_gen_to_stream;
use crate::exceptions::PoolTimeoutError;
use crate::middleware::Middleware;
use crate::proxy_config::NativeProxyConfig;
use crate::runtime::Runtime;
use crate::utils::{Body, Extensions, HeaderMapExt, MethodExt, UrlExt, copy_extensions, map_send_error};
use pyo3::coroutine::CancelHandle;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use reqwest::Client;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[pyclass]
pub struct NativeAsyncClient {
    client: Option<ClientWithMiddleware>,
    runtime: Arc<Runtime>,
    request_semaphore: Option<Arc<Semaphore>>,
    #[pyo3(get)]
    connect_timeout: Option<Duration>,
    #[pyo3(get)]
    proxy: Option<NativeProxyConfig>,
}

#[pymethods]
impl NativeAsyncClient {
    #[new]
    #[pyo3(signature = (
        *,
        total_timeout=None,
        connect_timeout=None,
        read_timeout=None,
        pool_idle_timeout=None,
        pool_max_idle_per_host=None,
        max_connections=None,
        http1=true,
        http2=true,
        root_certificates_der=None,
        proxy=None,
        middlewares=None
    ))]
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
        let runtime = Arc::new(Runtime::start()?);

        if let Some(middlewares) = middlewares {
            for middleware in middlewares {
                middleware_client = middleware_client.with(Middleware::new(py, middleware, runtime.clone())?);
            }
        }

        Ok(NativeAsyncClient {
            client: Some(middleware_client.build()),
            request_semaphore: max_connections.map(|limit| Arc::new(Semaphore::new(limit))),
            runtime,
            connect_timeout,
            proxy,
        })
    }

    #[pyo3(signature = (*, method, url, headers=None, body=None, stream=None, timeout=None, extensions=None))]
    async fn request(
        &self,
        method: MethodExt,
        url: UrlExt,
        headers: Option<HeaderMapExt>,
        body: Option<Body>,
        stream: Option<PyObject>,
        timeout: Option<Duration>,
        extensions: Option<Extensions>,
        #[pyo3(cancel_handle)] mut cancel: CancelHandle,
    ) -> PyResult<Py<Response>> {
        let client = self
            .client
            .clone()
            .ok_or_else(|| PyRuntimeError::new_err("Client is not initialized"))?;

        let request_semaphore = self.request_semaphore.clone();
        let connect_timeout = self.connect_timeout.clone();

        let join_handle = self.runtime.spawn(async move {
            let permit = if let Some(request_semaphore) = request_semaphore {
                Some(Self::limit_connections(request_semaphore, connect_timeout).await?)
            } else {
                None
            };

            let extensions_copy = extensions.clone();

            let mut response = Self::request_builder(client, method, url, headers, body, stream, timeout, extensions)?
                .send()
                .await
                .map_err(map_send_error)?;

            if let Some(extensions) = extensions_copy {
                copy_extensions(&extensions, response.extensions_mut());
            }

            Response::initialize(response, permit).await
        })?;

        tokio::select! {
            res = join_handle => {
                match res {
                    Ok(res) => res,
                    Err(e) => Err(PyRuntimeError::new_err(format!("Client was closed: {}", e))),
                }
            },
            _ = cancel.cancelled() => Err(PyRuntimeError::new_err("Request was cancelled")),
        }
    }

    fn close(&self) {
        self.runtime.close();
    }
}

impl NativeAsyncClient {
    fn request_builder(
        client: ClientWithMiddleware,
        method: MethodExt,
        url: UrlExt,
        headers: Option<HeaderMapExt>,
        body: Option<Body>,
        stream: Option<PyObject>,
        timeout: Option<Duration>,
        extensions: Option<Extensions>,
    ) -> PyResult<reqwest_middleware::RequestBuilder> {
        let url: reqwest::Url = url.try_into()?;

        if body.is_some() && stream.is_some() {
            return Err(PyValueError::new_err("Cannot set both body and stream"));
        }
        let body = match body {
            Some(Body::Str(body)) => Some(reqwest::Body::from(body)),
            Some(Body::Bytes(body)) => Some(reqwest::Body::from(body.into_inner())),
            None => match stream {
                Some(stream) => Some(reqwest::Body::wrap_stream(py_async_gen_to_stream(stream))),
                None => None,
            },
        };

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
        if let Some(extensions) = extensions {
            req_builder = req_builder.with_extension(extensions);
        }
        Ok(req_builder)
    }

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
