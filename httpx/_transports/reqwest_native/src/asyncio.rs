use async_stream::try_stream;
use futures_util::TryStream;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::{Bound, Py, PyAny, PyResult, Python, pyclass, pymethods};
use pyo3_bytes::PyBytes;
use tokio::sync::oneshot::Receiver;

fn get_event_loop(py: Python) -> PyResult<Bound<PyAny>> {
    static ONCE: GILOnceCell<Py<PyAny>> = GILOnceCell::new();
    ONCE.import(py, "asyncio", "get_event_loop")?.call0()
}

fn anext(py: Python, async_gen: Py<PyAny>) -> PyResult<Receiver<PyResult<PyObject>>> {
    static ONCE: GILOnceCell<Py<PyAny>> = GILOnceCell::new();
    let coro = ONCE.import(py, "builtins", "anext")?.call((async_gen,), None)?;
    py_coro_to_future(coro)
}

pub fn py_coro_to_future(py_coro: Bound<'_, PyAny>) -> PyResult<Receiver<PyResult<PyObject>>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let cb = FutCompletor { tx: Some(tx) };
    Python::with_gil(|py| {
        let py_task = get_event_loop(py)?.call_method1("create_task", (py_coro,))?;
        py_task.call_method1("add_done_callback", (cb,)).map(|_| ())
    })?;
    Ok(rx)
}

#[pyclass]
struct FutCompletor {
    tx: Option<tokio::sync::oneshot::Sender<PyResult<PyObject>>>,
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

    fn task_result(&self, task: Bound<PyAny>) -> PyResult<PyObject> {
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
    try_stream! {
        loop {
            let fut = Python::with_gil(|py| anext(py, async_gen.clone_ref(py)))?;
            let res = fut.await.map_err(|e| PyRuntimeError::new_err(format!("receive error: {}", e)))?;
            if let Err(e) = &res {
                let stop = Python::with_gil(|py| {
                    e.is_instance_of::<PyStopAsyncIteration>(py)
                });
                if stop {
                    break;
                }
            }
            let res = res?;
            yield Python::with_gil(|py| res.extract::<PyBytes>(py))?;
        }
    }
}
