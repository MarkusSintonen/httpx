use crate::asyncio::py_coro_to_future;
use crate::runtime::Runtime;
use crate::utils::{BytesExt, Extensions, HeaderMapExt, MethodExt, StatusCodeExt, UrlExt, VersionExt, copy_extensions};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::{IntoPyObjectExt, intern};
use pythonize::depythonize;
use serde::Deserialize;
use std::sync::Arc;

pub struct Middleware {
    handler: Py<PyAny>,
    runtime: Arc<Runtime>,
}

#[pyclass]
struct RequestWrapper {
    pub request: Option<reqwest::Request>,
}

#[pyclass]
struct ResponseWrapper {
    pub response: Option<reqwest::Response>,
}

#[derive(Deserialize)]
struct ResponseMock {
    status_code: Option<StatusCodeExt>,
    headers: Option<HeaderMapExt>,
    version: Option<VersionExt>,
    body: Option<Vec<u8>>,
    extensions: Option<Extensions>,
}

#[pyclass]
pub struct Next {
    req_sender: Option<tokio::sync::oneshot::Sender<(reqwest::Request, http::Extensions)>>,
    resp_receiver: Option<tokio::sync::oneshot::Receiver<(reqwest::Response, http::Extensions)>>,
}
#[pymethods]
impl Next {
    async fn run(&mut self, request: Py<RequestWrapper>, extensions: Extensions) -> PyResult<ResponseWrapper> {
        let req = Python::with_gil(|py| {
            request
                .borrow_mut(py)
                .request
                .take()
                .ok_or_else(|| PyRuntimeError::new_err("Request was already consumed"))
        })?;

        let mut ext = http::Extensions::new();
        ext.insert(extensions);

        let req_sender = self
            .req_sender
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("Request sender already consumed"))?;
        let resp_receiver = self
            .resp_receiver
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("Response receiver already consumed"))?;

        req_sender
            .send((req, ext))
            .map_err(|_| PyRuntimeError::new_err("Failed to send request to next middleware"))?;

        let (mut resp, ext) = resp_receiver
            .await
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to receive response from next middleware: {}", e)))?;

        if let Some(ext) = ext.get::<Extensions>() {
            copy_extensions(ext, resp.extensions_mut());
        }

        Ok(ResponseWrapper { response: Some(resp) })
    }

    fn create_response<'py>(&mut self, py: Python<'py>, response_mock: ResponseMock) -> PyResult<Py<ResponseWrapper>> {
        let resp: reqwest::Response = response_mock.try_into()?;
        Py::new(py, ResponseWrapper { response: Some(resp) })
    }
}

#[async_trait::async_trait]
impl reqwest_middleware::Middleware for Middleware {
    async fn handle(
        &self,
        http_request: reqwest::Request,
        http_extensions: &mut http::Extensions,
        next: reqwest_middleware::Next<'_>,
    ) -> reqwest_middleware::Result<reqwest::Response> {
        let req = RequestWrapper::from(http_request);
        let ext = Extensions::from(&*http_extensions);

        let (req_tx, req_rx) = tokio::sync::oneshot::channel::<(reqwest::Request, http::Extensions)>();
        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel::<(reqwest::Response, http::Extensions)>();
        let next_wrap = Next {
            req_sender: Some(req_tx),
            resp_receiver: Some(resp_rx),
        };

        let fut = Python::with_gil(|py| {
            let coro = self
                .handler
                .call_method1(py, intern!(py, "handle"), (req, ext, next_wrap))?;
            py_coro_to_future(coro)
        })
        .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;

        let join_handle = self
            .runtime
            .spawn(fut)
            .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;

        let (req, mut ext) = req_rx
            .await
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to receive response from next middleware: {}", e)))
            .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;

        let resp = next.run(req, &mut ext).await?;

        resp_tx
            .send((resp, ext))
            .map_err(|_| PyRuntimeError::new_err("Failed to send request to next middleware"))
            .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;

        let resp = join_handle
            .await
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to execute middleware: {}", e)))
            .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;

        Python::with_gil(|py| {
            resp?
                .downcast_bound::<ResponseWrapper>(py)?
                .try_borrow_mut()?
                .response
                .take()
                .ok_or_else(|| PyRuntimeError::new_err("Response was already consumed"))
        })
        .map_err(reqwest_middleware::Error::middleware::<PyErr>)
    }
}

impl Middleware {
    pub fn new(py: Python, handler: Bound<PyAny>, runtime: Arc<Runtime>) -> PyResult<Self> {
        if !handler.hasattr(intern!(py, "handle"))? {
            return Err(PyValueError::new_err("Middleware must have handle method"));
        }
        Ok(Middleware {
            handler: handler.into_py_any(py)?,
            runtime,
        })
    }
}

#[pymethods]
impl RequestWrapper {
    fn get_method(&self) -> PyResult<MethodExt> {
        Ok(self.try_get_request()?.method().clone().into())
    }

    fn set_method(&mut self, value: MethodExt) -> PyResult<()> {
        *self.try_mut_request()?.method_mut() = value.0;
        Ok(())
    }

    fn get_url(&self) -> PyResult<UrlExt> {
        self.try_get_request()?.url().clone().try_into()
    }

    fn set_url(&mut self, value: UrlExt) -> PyResult<()> {
        *self.try_mut_request()?.url_mut() = value.try_into()?;
        Ok(())
    }

    fn get_headers(&self) -> PyResult<HeaderMapExt> {
        Ok(self.try_get_request()?.headers().clone().into())
    }

    fn set_headers(&mut self, value: HeaderMapExt) -> PyResult<()> {
        *self.try_mut_request()?.headers_mut() = value.0;
        Ok(())
    }

    fn get_body(&self) -> PyResult<Option<BytesExt>> {
        let body = self
            .try_get_request()?
            .body()
            .map(|b| b.as_bytes())
            .flatten()
            .map(|b| BytesExt::from(b.to_vec()));
        Ok(body)
    }

    fn set_body(&mut self, value: Option<BytesExt>) -> PyResult<()> {
        if let Some(value) = value {
            *self.try_mut_request()?.body_mut() = Some(reqwest::Body::from(value.0));
        } else {
            *self.try_mut_request()?.body_mut() = None;
        }
        Ok(())
    }
}
impl RequestWrapper {
    fn try_get_request(&self) -> PyResult<&reqwest::Request> {
        self.request
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Request was already consumed"))
    }

    fn try_mut_request(&mut self) -> PyResult<&mut reqwest::Request> {
        if let Some(req) = self.request.as_mut() {
            Ok(req)
        } else {
            Err(PyRuntimeError::new_err("Request was already consumed"))
        }
    }
}
impl From<reqwest::Request> for RequestWrapper {
    fn from(value: reqwest::Request) -> Self {
        RequestWrapper { request: Some(value) }
    }
}

#[pymethods]
impl ResponseWrapper {
    fn get_status(&self) -> PyResult<StatusCodeExt> {
        Ok(self.try_get_response()?.status().into())
    }

    fn get_url(&self) -> PyResult<UrlExt> {
        self.try_get_response()?.url().clone().try_into()
    }

    fn get_version(&self) -> PyResult<VersionExt> {
        Ok(self.try_get_response()?.version().into())
    }

    fn get_headers(&self) -> PyResult<HeaderMapExt> {
        Ok(self.try_get_response()?.headers().clone().into())
    }

    fn set_headers(&mut self, value: HeaderMapExt) -> PyResult<()> {
        *self.try_mut_response()?.headers_mut() = value.0;
        Ok(())
    }

    fn get_content_length(&self) -> PyResult<Option<u64>> {
        Ok(self.try_get_response()?.content_length())
    }

    fn get_extensions(&self) -> PyResult<Extensions> {
        Ok(self.try_get_response()?.extensions().into())
    }

    fn set_extensions(&mut self, value: Extensions) -> PyResult<()> {
        self.try_mut_response()?.extensions_mut().insert(value);
        Ok(())
    }
}
impl ResponseWrapper {
    fn try_get_response(&self) -> PyResult<&reqwest::Response> {
        self.response
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Response was already consumed"))
    }

    fn try_mut_response(&mut self) -> PyResult<&mut reqwest::Response> {
        if let Some(req) = self.response.as_mut() {
            Ok(req)
        } else {
            Err(PyRuntimeError::new_err("Response was already consumed"))
        }
    }
}

impl<'py> FromPyObject<'py> for ResponseMock {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(depythonize(ob)?)
    }
}
impl TryInto<reqwest::Response> for ResponseMock {
    type Error = PyErr;
    fn try_into(self) -> PyResult<reqwest::Response> {
        let mut res = http::Response::builder();
        if let Some(status_code) = &self.status_code {
            res = res.status(status_code.0);
        }
        if let Some(headers) = &self.headers {
            for (k, v) in headers.0.iter() {
                res = res.header(k, v);
            }
        }
        if let Some(version) = &self.version {
            res = res.version(version.0);
        }
        if let Some(extensions) = &self.extensions {
            res = res.extension(extensions.clone());
        }
        let res = res
            .body(self.body.clone().unwrap_or_default())
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(reqwest::Response::from(res))
    }
}
