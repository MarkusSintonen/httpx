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
    PoolTimeoutError, ReadConnectionError, ReadError, ReadTimeoutError, SendConnectionError, SendError,
    SendTimeoutError,
};
use crate::proxy_config::ProxyConfig;
use pyo3::prelude::*;

#[pymodule]
fn reqwest_native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<AsyncClient>()?;
    module.add_class::<Response>()?;
    module.add_class::<ProxyConfig>()?;

    module.add("SendError", module.py().get_type::<SendError>())?;
    module.add("SendConnectionError", module.py().get_type::<SendConnectionError>())?;
    module.add("SendTimeoutError", module.py().get_type::<SendTimeoutError>())?;
    module.add("PoolTimeoutError", module.py().get_type::<PoolTimeoutError>())?;

    module.add("ReadError", module.py().get_type::<ReadError>())?;
    module.add("ReadConnectionError", module.py().get_type::<ReadConnectionError>())?;
    module.add("ReadTimeoutError", module.py().get_type::<ReadTimeoutError>())?;

    Ok(())
}
