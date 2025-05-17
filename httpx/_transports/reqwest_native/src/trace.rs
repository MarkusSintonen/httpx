use crate::utils::{
    BytesExt, ExtensionValue, Extensions, extensions_from_dict, headers_to_bytes,
    http_extensions_to_dict, http_version_str,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3::{Bound, IntoPyObjectExt, PyAny, PyObject, PyResult, Python, intern};
use reqwest::{Request, Response};
use reqwest_middleware::{Middleware, Next};

pub struct TraceMiddleware {
    tracer: Option<PyObject>,
}

#[pyclass]
#[derive(Clone)]
pub struct RequestTraceInfo {
    #[pyo3(get)]
    method: String,
    #[pyo3(get)]
    url: String,
}

#[pyclass]
#[derive(Clone)]
pub struct ResponseTraceInfo {
    #[pyo3(get)]
    status_code: u16,
    #[pyo3(get)]
    headers: Vec<(Vec<u8>, Vec<u8>)>,
    #[pyo3(get)]
    version: String,
}

#[pyclass]
pub struct RequestTraceData {
    #[pyo3(get)]
    request: RequestTraceInfo,
    #[pyo3(get)]
    extensions: Py<PyDict>,
}

#[pyclass]
#[derive(Clone)]
pub struct ResponseErrorTraceData {
    #[pyo3(get)]
    error_message: String,
    // TODO: Add more fields if needed
}

#[pyclass]
pub struct ResponseTraceData {
    #[pyo3(get)]
    request: RequestTraceInfo,
    #[pyo3(get)]
    response: Option<ResponseTraceInfo>,
    #[pyo3(get)]
    error: Option<ResponseErrorTraceData>,
    #[pyo3(get)]
    extensions: Py<PyDict>,
}

#[async_trait::async_trait]
impl Middleware for TraceMiddleware {
    async fn handle(
        &self,
        request: Request,
        http_extensions: &mut http::Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        let tracer = self.on_request_start(&request, http_extensions).await?;

        let mut result = next.run(request, http_extensions).await;

        self.on_request_end(tracer, &result, http_extensions)
            .await?;

        if let Ok(result) = &mut result {
            let version: BytesExt = http_version_str(result.version()).into_bytes().into();
            let ext = result.extensions_mut().get_or_insert_with(Extensions::new);
            ext.insert("http_version".to_string(), ExtensionValue::Bytes(version));
            if let Some(req_ext) = http_extensions.get::<Extensions>() {
                for (k, v) in req_ext.into_iter() {
                    ext.insert(k.clone(), v.clone());
                }
            }
        }

        result
    }
}

impl TraceMiddleware {
    pub fn new(py: Python, tracer: Option<Bound<PyAny>>) -> PyResult<Self> {
        if let Some(tracer) = tracer {
            if !tracer.hasattr("on_request_start")? {
                return Err(PyValueError::new_err(
                    "Tracer must have on_request_start method",
                ));
            }
            if !tracer.hasattr("on_request_end")? {
                return Err(PyValueError::new_err(
                    "Tracer must have on_request_end method",
                ));
            }
            Ok(TraceMiddleware {
                tracer: Some(tracer.into_py_any(py)?),
            })
        } else {
            Ok(TraceMiddleware { tracer: None })
        }
    }

    async fn on_request_start(
        &self,
        request: &Request,
        http_extensions: &mut http::Extensions,
    ) -> reqwest_middleware::Result<Option<(&PyObject, RequestTraceInfo)>> {
        let Some(tracer) = &self.tracer else {
            return Ok(None);
        };

        let req_info = RequestTraceInfo::from(request);
        let ext_dict =
            Python::with_gil(|py| Ok(http_extensions_to_dict(py, http_extensions)?.unbind()))
                .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;

        let fut = Python::with_gil(|py| {
            let req_data = RequestTraceData {
                request: req_info.clone(),
                extensions: ext_dict.clone_ref(py), // Can be mutated in Python callback
            };
            let coro = tracer
                .bind(py)
                .call_method1(intern!(py, "on_request_start"), (req_data,))?;
            pyo3_async_runtimes::tokio::into_future(coro)
        })
        .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;

        fut.await
            .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;

        let ext = Python::with_gil(|py| extensions_from_dict(ext_dict.bind(py)))
            .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;
        http_extensions.insert(ext);

        Ok(Some((tracer, req_info)))
    }

    async fn on_request_end(
        &self,
        tracer: Option<(&PyObject, RequestTraceInfo)>,
        result: &reqwest_middleware::Result<Response>,
        http_extensions: &mut http::Extensions,
    ) -> reqwest_middleware::Result<()> {
        let Some((tracer, request_info)) = tracer else {
            return Ok(());
        };

        let ext_dict =
            Python::with_gil(|py| Ok(http_extensions_to_dict(py, http_extensions)?.unbind()))
                .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;

        let fut = Python::with_gil(|py| {
            let resp_data = ResponseTraceData {
                request: request_info,
                response: result
                    .as_ref()
                    .ok()
                    .map(ResponseTraceInfo::try_from)
                    .transpose()?,
                error: result.as_ref().err().map(ResponseErrorTraceData::from),
                extensions: ext_dict.clone_ref(py), // Can be mutated in Python callback
            };
            let coro = tracer
                .bind(py)
                .call_method1(intern!(py, "on_request_end"), (resp_data,))?;
            pyo3_async_runtimes::tokio::into_future(coro)
        })
        .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;

        fut.await
            .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;

        let ext = Python::with_gil(|py| extensions_from_dict(ext_dict.bind(py)))
            .map_err(reqwest_middleware::Error::middleware::<PyErr>)?;
        http_extensions.insert(ext);
        Ok(())
    }
}

impl From<&Request> for RequestTraceInfo {
    fn from(request: &Request) -> Self {
        RequestTraceInfo {
            method: request.method().to_string(),
            url: request.url().to_string(),
        }
    }
}

impl TryFrom<&Response> for ResponseTraceInfo {
    type Error = PyErr;
    fn try_from(response: &Response) -> Result<Self, PyErr> {
        let res = ResponseTraceInfo {
            status_code: response.status().as_u16(),
            headers: headers_to_bytes(response.headers()),
            version: http_version_str(response.version()),
        };
        Ok(res)
    }
}

impl From<&reqwest_middleware::Error> for ResponseErrorTraceData {
    fn from(err: &reqwest_middleware::Error) -> Self {
        ResponseErrorTraceData {
            error_message: format!("{}", err),
        }
    }
}
