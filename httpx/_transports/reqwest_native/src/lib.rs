// rustimport:pyo3

mod async_client;
mod async_response;
mod exceptions;

use crate::async_client::NativeAsyncClient;
use crate::async_response::NativeAsyncResponse;
use crate::exceptions::{
    BadHeaderError, BadMethodError, BadUrlError, PoolTimeoutError, ReadConnectionError,
    ReadTimeoutError, ReadUnknownError, SendConnectionError, SendTimeoutError, SendUnknownError,
};
use pyo3::prelude::*;

#[pymodule]
fn reqwest_native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeAsyncClient>()?;
    module.add_class::<NativeAsyncResponse>()?;

    module.add("BadMethodError", module.py().get_type::<BadMethodError>())?;
    module.add("BadUrlError", module.py().get_type::<BadUrlError>())?;
    module.add("BadHeaderError", module.py().get_type::<BadHeaderError>())?;

    module.add(
        "SendConnectionError",
        module.py().get_type::<SendConnectionError>(),
    )?;
    module.add(
        "SendTimeoutError",
        module.py().get_type::<SendTimeoutError>(),
    )?;
    module.add(
        "SendUnknownError",
        module.py().get_type::<SendUnknownError>(),
    )?;

    module.add(
        "PoolTimeoutError",
        module.py().get_type::<PoolTimeoutError>(),
    )?;

    module.add(
        "ReadConnectionError",
        module.py().get_type::<ReadConnectionError>(),
    )?;
    module.add(
        "ReadTimeoutError",
        module.py().get_type::<ReadTimeoutError>(),
    )?;
    module.add(
        "ReadUnknownError",
        module.py().get_type::<ReadUnknownError>(),
    )?;

    Ok(())
}
