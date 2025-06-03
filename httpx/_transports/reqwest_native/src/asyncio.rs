use crate::runtime::Runtime;
use async_stream::try_stream;
use futures_util::TryStream;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::{Bound, IntoPyObject, Py, PyAny, PyResult, Python, pyclass, pymethods, intern};
use pyo3_bytes::PyBytes;
use tokio::sync::oneshot::Receiver;

fn get_event_loop(py: Python) -> PyResult<Bound<PyAny>> {
    static ONCE: GILOnceCell<Py<PyAny>> = GILOnceCell::new();
    ONCE.import(py, "asyncio", "get_event_loop")?.call0()
}

fn event_loop_wake_up<'py>(py: Python<'py>, ev_loop: Bound<'py, PyAny>) -> PyResult<()> {
    run_coroutine_threadsafe(py, sleep(py, 0.0)?, ev_loop)
}

fn anext(py: Python, async_gen: Py<PyAny>) -> PyResult<Receiver<PyResult<PyObject>>> {
    static ONCE: GILOnceCell<Py<PyAny>> = GILOnceCell::new();
    let coro = ONCE.import(py, "builtins", "anext")?.call((async_gen,), None)?;
    py_coro_to_future(coro)
}

fn run_coroutine_threadsafe(py: Python, coro: Bound<PyAny>, ev_loop: Bound<PyAny>) -> PyResult<()> {
    static ONCE: GILOnceCell<Py<PyAny>> = GILOnceCell::new();
    ONCE.import(py, "asyncio", "run_coroutine_threadsafe")?.call((coro, ev_loop), None).map(|_| ())
}

fn sleep(py: Python, secs: f64) -> PyResult<Bound<PyAny>> {
    static ONCE: GILOnceCell<Py<PyAny>> = GILOnceCell::new();
    ONCE.import(py, "asyncio", "sleep")?.call((secs,), None)
}

pub fn resolve_coro<'py, T>(py: Python<'py>, result: T) -> PyResult<Bound<'py, PyAny>>
    where T: IntoPyObject<'py>,
{
    let py_loop = get_event_loop(py)?;
    let py_fut = py_loop.call_method0("create_future")?;
    py_fut.call_method1("set_result", (result,))?;
    Ok(py_fut)
}

pub fn future_to_coro<'py, F, T>(py: Python<'py>, runtime: &Runtime, fut: F) -> PyResult<Bound<'py, PyAny>>
where
    F: Future<Output = PyResult<T>> + Send + 'static,
    T: for<'p> IntoPyObject<'p> + Send + 'static,
{
    let py_loop = get_event_loop(py)?.unbind();
    let py_fut = py_loop.call_method0(py, intern!(py, "create_future"))?;
    let py_tx = py_fut.clone_ref(py);
    let cb = PyFutCompletor { py_tx: Some(py_tx) };

    let _ = runtime.spawn(async move {
        let res = fut.await;
        Python::with_gil(|py| match res {
            Ok(res) => py_loop
                .call_method1(py, intern!(py, "call_soon_threadsafe"), (cb, intern!(py, "set_result"), res))
                .unwrap(),
            Err(e) => py_loop
                .call_method1(py, intern!(py, "call_soon_threadsafe"), (cb, intern!(py, "set_exception"), e))
                .unwrap(),
        })
        // Python::with_gil(|py| {
        //     match res {
        //         Ok(res) => py_tx.call_method1(py, "set_result", (res,)).unwrap(),
        //         Err(e) => py_tx.call_method1(py, "set_exception", (e,)).unwrap(),
        //     };
        //     event_loop_wake_up(py, py_loop.into_bound(py)).unwrap();
        // })
    });

    Ok(py_fut.into_bound(py))
}

#[pyclass]
struct PyFutCompletor {
    py_tx: Option<Py<PyAny>>,
}
#[pymethods]
impl PyFutCompletor {
    fn __call__(&mut self, py: Python, action: &str, value: Bound<PyAny>) -> PyResult<()> {
        self.py_tx
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("py tx already consumed"))?
            .call_method1(py, action, (value,))
            .map(|_| ())
    }
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
