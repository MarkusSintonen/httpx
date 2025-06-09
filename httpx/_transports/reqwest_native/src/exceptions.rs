use pyo3::create_exception;
use pyo3::exceptions::PyException;

create_exception!(module, SendError, PyException);
create_exception!(module, SendConnectionError, SendError);
create_exception!(module, SendTimeoutError, SendError);
create_exception!(module, PoolTimeoutError, SendError);

create_exception!(module, ReadError, PyException);
create_exception!(module, ReadConnectionError, ReadError);
create_exception!(module, ReadTimeoutError, ReadError);
