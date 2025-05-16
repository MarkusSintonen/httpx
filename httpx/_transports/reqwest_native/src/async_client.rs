use crate::async_response::NativeAsyncResponse;
use crate::exceptions::{
    BadHeaderError, BadUrlError, PoolTimeoutError, SendConnectionError, SendTimeoutError,
    SendUnknownError,
};
use crate::proxy_config::NativeProxyConfig;
use crate::utils::{parse_method, parse_url};
use futures_util::stream::StreamExt;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Body, Client};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[pyclass]
pub struct NativeAsyncClient {
    client: Option<Client>,
    request_semaphore: Option<Arc<Semaphore>>,
    connect_timeout: Option<Duration>,
    #[pyo3(get)]
    proxy: Option<NativeProxyConfig>,
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
    ) -> PyResult<Self> {
        if !http1 && !http2 {
            return Err(PyValueError::new_err(
                "At least one of http1 or http2 must be true",
            ));
        }
        if let Some(max_conns) = max_connections {
            if max_conns == 0 {
                return Err(PyValueError::new_err(
                    "max_connections must be greater than 0",
                ));
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
                client =
                    client.add_root_certificate(reqwest::Certificate::from_der(&cert).map_err(
                        |e| PyValueError::new_err(format!("Invalid certificate: {}", e)),
                    )?);
            }
        }
        if let Some(proxy) = &proxy {
            client = client.proxy(proxy.build_reqwest_proxy()?);
        }

        let client = client
            .build()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create HTTP client: {}", e)))?;

        Ok(NativeAsyncClient {
            client: Some(client),
            request_semaphore: max_connections.map(|limit| Arc::new(Semaphore::new(limit))),
            connect_timeout,
            proxy,
        })
    }

    fn request<'py>(
        &self,
        py: Python<'py>,
        method: String,
        url: String,
        headers: Option<Vec<(Vec<u8>, Vec<u8>)>>,
        content: Option<Bound<'py, PyAny>>,
        timeout: Option<Duration>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let client = self
            .client
            .clone()
            .ok_or_else(|| PyRuntimeError::new_err("Client is not initialized"))?;

        let method = parse_method(method)?;
        let url = parse_url(url)?;
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(BadUrlError::new_err(format!(
                "Invalid URL scheme: {}",
                url.scheme()
            )));
        }

        let body = content
            .map(|content| {
                if content.is_exact_instance_of::<PyBytes>() {
                    let py_bytes = unsafe { content.downcast_unchecked::<PyBytes>() }.as_bytes();
                    py.allow_threads(|| Ok(Body::from(py_bytes.to_vec())))
                } else {
                    pyo3_async_runtimes::tokio::into_stream_v2(content).map(|py_stream| {
                        let stream = py_stream
                            .map(|chunk| Python::with_gil(|py| chunk.extract::<Vec<u8>>(py)));
                        Body::wrap_stream(stream)
                    })
                }
            })
            .transpose()?;

        let request_semaphore = self.request_semaphore.clone();
        let connect_timeout = self.connect_timeout.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let permit = if let Some(request_semaphore) = request_semaphore {
                Some(Self::limit_connections(request_semaphore, connect_timeout).await?)
            } else {
                None
            };

            let mut req_builder = client.request(method, url);
            if let Some(body) = body {
                req_builder = req_builder.body(body);
            }
            if let Some(headers) = headers {
                for (header_key, header_value) in headers.into_iter() {
                    let header_name = HeaderName::from_bytes(&header_key)
                        .map_err(|_| BadHeaderError::new_err("Invalid header key"))?;
                    let header_value = HeaderValue::from_bytes(&header_value)
                        .map_err(|_| BadHeaderError::new_err("Invalid header value"))?;
                    req_builder = req_builder.header(header_name, header_value);
                }
            }
            if let Some(timeout) = timeout {
                req_builder = req_builder.timeout(timeout);
            }

            let request = req_builder
                .build()
                .map_err(|e| PyRuntimeError::new_err(format!("Invalid request: {}", e)))?;

            let response = client
                .execute(request)
                .await
                .map_err(Self::map_send_error)?;

            NativeAsyncResponse::new(response, permit)
        })
    }

    fn close<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let mut client = self.client.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            client.take(); // Drop the client
            Ok(())
        })
    }
}

impl NativeAsyncClient {
    fn map_send_error(error: reqwest::Error) -> PyErr {
        if error.is_connect() {
            SendConnectionError::new_err(format!("Connection error on send: {}", error))
        } else if error.is_timeout() {
            SendTimeoutError::new_err(format!("Timeout on send: {}", error))
        } else {
            SendUnknownError::new_err(format!("Unknown failure on send: {}", error))
        }
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
