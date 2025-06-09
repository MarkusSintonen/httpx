use async_stream::try_stream;
use futures_util::TryStream;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::PyNone;
use pyo3::{Bound, Py, PyAny, PyResult, Python, pyclass, pymethods};
use pyo3_bytes::PyBytes;

pub fn py_coro_to_future(py_coro: Py<PyAny>) -> PyResult<impl Future<Output = PyResult<Py<PyAny>>>> {
    static EV_LOOP_ONCE: GILOnceCell<Py<PyAny>> = GILOnceCell::new();

    let (tx, rx) = tokio::sync::oneshot::channel();
    let cb = FutCompletor { tx: Some(tx) };

    Python::with_gil(|py| {
        let ev_loop = EV_LOOP_ONCE.import(py, "asyncio", "get_event_loop")?.call0()?;
        let py_task = ev_loop.call_method1("create_task", (py_coro,))?;
        py_task.call_method1("add_done_callback", (cb,)).map(|_| ())
    })?;

    Ok(async move {
        match rx.await {
            Ok(result) => result,
            Err(e) => Err(PyRuntimeError::new_err(format!("Failed to receive task result: {}", e))),
        }
    })
}

#[pyclass]
struct FutCompletor {
    tx: Option<tokio::sync::oneshot::Sender<PyResult<Py<PyAny>>>>,
}
#[pymethods]
impl FutCompletor {
    fn __call__(&mut self, task: Bound<PyAny>) -> PyResult<()> {
        self.tx
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("tx already consumed"))?
            .send(self.task_result(task))
            .map_err(|_| PyRuntimeError::new_err("Failed to send task result"))
    }

    fn task_result(&self, task: Bound<PyAny>) -> PyResult<Py<PyAny>> {
        match task.call_method0("exception") {
            Ok(task_exc) => {
                if task_exc.is_none() {
                    task.call_method0("result").map(|res| res.unbind())
                } else {
                    Err(PyErr::from_value(task_exc))
                }
            }
            Err(err) => Err(err),
        }
    }
}

pub fn py_async_gen_to_stream(async_gen: Py<PyAny>) -> impl TryStream<Ok = PyBytes, Error = PyErr> + 'static {
    static ONCE_ANEXT: GILOnceCell<Py<PyAny>> = GILOnceCell::new();

    try_stream! {
        loop {
            let fut = Python::with_gil(|py| {
                let async_gen = async_gen.clone_ref(py);
                let anext = ONCE_ANEXT.import(py, "builtins", "anext")?;
                let coro = anext.call((async_gen, PyNone::get(py)), None)?;
                Ok::<_, PyErr>(py_coro_to_future(coro.unbind()))
            })??;

            let res = fut.await?;

            if let Some(bytes) = Python::with_gil(|py| res.extract::<Option<PyBytes>>(py))? {
                yield bytes;
            } else {
                break;
            };
        }
    }
}
