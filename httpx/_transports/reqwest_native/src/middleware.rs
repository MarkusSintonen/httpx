use crate::asyncio::py_coro_to_future;
use crate::http_types::{Extensions, HeaderMapExt, MethodExt, RequestBody, StatusCodeExt, UrlExt, VersionExt};
use crate::utils::map_send_error;
use pyo3::exceptions::PyRuntimeError;
use pyo3::intern;
use pyo3::prelude::*;
use pythonize::depythonize;
use serde::Deserialize;
use std::sync::Arc;

#[pyclass]
pub struct RequestWrapper {
    request: Option<reqwest::Request>,
    extensions: Option<Extensions>,
    #[pyo3(get, set)]
    body: Option<Py<RequestBody>>,
}

#[pyclass]
pub struct ResponseWrapper {
    response: Option<reqwest::Response>,
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
    client: Arc<reqwest::Client>,
    middlewares: Arc<Vec<Py<PyAny>>>,
    current: usize,
}
#[pymethods]
impl Next {
    async fn run(&self, request: Py<RequestWrapper>) -> PyResult<Py<ResponseWrapper>> {
        if self.current < self.middlewares.len() {
            let cur = &self.middlewares[self.current];
            let next = Python::with_gil(|py| {
                let next = Next {
                    client: self.client.clone(),
                    middlewares: self.middlewares.clone(),
                    current: self.current + 1,
                };
                Py::new(py, next)
            })?;

            let fut = Python::with_gil(|py| {
                let coro = cur.call_method1(py, intern!(py, "handle"), (request, next))?;
                py_coro_to_future(coro)
            })?;

            let res = fut.await?;

            Python::with_gil(|py| Ok::<_, PyErr>(res.into_bound(py).downcast_into_exact::<ResponseWrapper>()?.unbind()))
        } else {
            let (req, ext) = Python::with_gil(|py| {
                let mut request = request.try_borrow_mut(py)?;
                let req = request
                    .request
                    .take()
                    .ok_or_else(|| PyRuntimeError::new_err("Request was already consumed"))?;
                let ext = request.extensions.take();
                Ok::<_, PyErr>((req, ext))
            })?;

            let mut resp = self.client.execute(req).await.map_err(map_send_error)?;

            resp.extensions_mut().insert(ext);

            Python::with_gil(|py| Py::new(py, ResponseWrapper { response: Some(resp) }))
        }
    }

    fn create_response<'py>(&mut self, py: Python<'py>, response_mock: ResponseMock) -> PyResult<Py<ResponseWrapper>> {
        let resp: reqwest::Response = response_mock.try_into()?;
        Py::new(py, ResponseWrapper { response: Some(resp) })
    }
}
impl Next {
    pub async fn process(
        client: Arc<reqwest::Client>,
        middlewares: Arc<Vec<Py<PyAny>>>,
        request: reqwest::Request,
        body: Option<Py<RequestBody>>,
        extensions: Option<Extensions>,
    ) -> PyResult<reqwest::Response> {
        let req = RequestWrapper::new(request, body, extensions);
        let req = Python::with_gil(|py| Py::new(py, req))?;

        let resp = Next {
            client,
            middlewares,
            current: 0,
        }
        .run(req)
        .await?;

        Python::with_gil(|py| {
            resp.try_borrow_mut(py)?
                .response
                .take()
                .ok_or_else(|| PyRuntimeError::new_err("Response was already consumed"))
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

    fn get_extensions(&self) -> Option<Extensions> {
        self.extensions.clone()
    }

    fn set_extensions(&mut self, value: Option<Extensions>) {
        self.extensions = value;
    }
}
impl RequestWrapper {
    pub fn new(request: reqwest::Request, body: Option<Py<RequestBody>>, extensions: Option<Extensions>) -> Self {
        RequestWrapper {
            request: Some(request),
            extensions,
            body,
        }
    }

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
