use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};

create_exception!(module, BadMethodError, PyValueError);
create_exception!(module, BadUrlError, PyValueError);
create_exception!(module, BadHeaderError, PyValueError);

create_exception!(module, SendConnectionError, PyException);
create_exception!(module, SendTimeoutError, PyException);
create_exception!(module, SendUnknownError, PyException);

create_exception!(module, PoolTimeoutError, PyException);

create_exception!(module, ReadConnectionError, PyException);
create_exception!(module, ReadTimeoutError, PyException);
create_exception!(module, ReadUnknownError, PyException);
