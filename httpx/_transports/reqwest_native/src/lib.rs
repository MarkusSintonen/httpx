// rustimport:pyo3

mod async_client;
mod async_response;
mod asyncio;
mod exceptions;
mod http_types;
mod middleware;
mod proxy_config;
mod runtime;
mod utils;

use crate::async_client::AsyncClient;
use crate::async_response::Response;
use crate::exceptions::{
    PoolTimeoutError, ReadBodyError, ReadError, ReadTimeoutError, RequestError, SendBodyError, SendConnectionError,
    SendError, SendTimeoutError,
};
use crate::proxy_config::ProxyConfig;
use pyo3::prelude::*;

#[pymodule]
fn reqwest_native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<AsyncClient>()?;
    module.add_class::<Response>()?;
    module.add_class::<ProxyConfig>()?;

    module.add("RequestError", module.py().get_type::<RequestError>())?;
    module.add("SendError", module.py().get_type::<SendError>())?;
    module.add("SendConnectionError", module.py().get_type::<SendConnectionError>())?;
    module.add("SendBodyError", module.py().get_type::<SendBodyError>())?;
    module.add("SendTimeoutError", module.py().get_type::<SendTimeoutError>())?;
    module.add("PoolTimeoutError", module.py().get_type::<PoolTimeoutError>())?;
    module.add("ReadError", module.py().get_type::<ReadError>())?;
    module.add("ReadBodyError", module.py().get_type::<ReadBodyError>())?;
    module.add("ReadTimeoutError", module.py().get_type::<ReadTimeoutError>())?;

    Ok(())
}
