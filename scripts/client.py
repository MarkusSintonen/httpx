import asyncio
import os
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from typing import Any, Callable, Coroutine, Iterator, List

import aiohttp
import matplotlib.pyplot as plt  # type: ignore[import-untyped]
import pyinstrument
import urllib3
from matplotlib.axes import Axes  # type: ignore[import-untyped]

from httpx._transports.reqwest import AsyncReqwestHTTPTransport
from httpx import Limits, Request
from scripts.server import RESP

PORT = 1234
URL = f"http://localhost:{PORT}/req"
REPEATS = 10
REQUESTS = 500
CONCURRENCY = 10
POOL_LIMIT = 100
PROFILE = False
os.environ["HTTPCORE_PREFER_ANYIO"] = "0"


def duration(start: float) -> float:
    return round((time.monotonic() - start) * 1000, 2)


@contextmanager
def profile():
    if not PROFILE:
        yield
        return
    with pyinstrument.Profiler() as profiler:
        yield
    profiler.open_in_browser()


async def run_async_requests(axis: Axes) -> None:
    async def gather_limited_concurrency(
            coros: Iterator[Coroutine[Any, Any, Any]], concurrency: int = CONCURRENCY
    ) -> None:
        sem = asyncio.Semaphore(concurrency)

        async def coro_with_sem(coro: Coroutine[Any, Any, Any]) -> None:
            async with sem:
                await coro

        await asyncio.gather(*(coro_with_sem(c) for c in coros))

    async def httpx_get(
        client: AsyncReqwestHTTPTransport, timings: List[float]
    ) -> None:
        start = time.monotonic()
        res = await client.handle_async_request("GET", URL)
        body = await res.read()
        assert len(body) == len(RESP), f"len={len(body)}"
        assert res.status_code == 200, f"status_code={res.status_code}"
        await res.aclose()
        timings.append(duration(start))

    async def aiohttp_get(session: aiohttp.ClientSession, timings: List[float]) -> None:
        start = time.monotonic()
        async with session.request("GET", URL) as res:
            body = await res.read()
            assert len(body) == len(RESP)
            assert res.status == 200, f"status={res.status}"
        timings.append(duration(start))

    async with AsyncReqwestHTTPTransport(limits=Limits(max_connections=POOL_LIMIT)) as client:
        # warmup
        await gather_limited_concurrency(
            (httpx_get(client, []) for _ in range(REQUESTS)), CONCURRENCY * 2
        )

        timings: List[float] = []
        start = time.monotonic()
        with profile():
            for _ in range(REPEATS):
                await gather_limited_concurrency(
                    (httpx_get(client, timings) for _ in range(REQUESTS))
                )
        axis.plot(
            [*range(len(timings))], timings, label=f"httpx (tot={duration(start)}ms)"
        )

    connector = aiohttp.TCPConnector(limit=POOL_LIMIT)
    async with aiohttp.ClientSession(connector=connector) as session:
        # warmup
        await gather_limited_concurrency(
            (aiohttp_get(session, []) for _ in range(REQUESTS)), CONCURRENCY * 2
        )

        timings = []
        start = time.monotonic()
        for _ in range(REPEATS):
            await gather_limited_concurrency(
                (aiohttp_get(session, timings) for _ in range(REQUESTS))
            )
        axis.plot(
            [*range(len(timings))], timings, label=f"aiohttp (tot={duration(start)}ms)"
        )


def run_sync_requests(axis: Axes) -> None:
    def run_in_executor(
            fns: Iterator[Callable[[], None]], executor: ThreadPoolExecutor
    ) -> None:
        futures = [executor.submit(fn) for fn in fns]
        for future in futures:
            future.result()

    def httpcore_get(pool: httpcore.ConnectionPool, timings: List[float]) -> None:
        start = time.monotonic()
        res = pool.request("GET", URL)
        assert len(res.read()) == 2000
        assert res.status == 200, f"status_code={res.status}"
        timings.append(duration(start))

    def urllib3_get(pool: urllib3.HTTPConnectionPool, timings: List[float]) -> None:
        start = time.monotonic()
        res = pool.request("GET", "/req")
        assert len(res.data) == 2000
        assert res.status == 200, f"status={res.status}"
        timings.append(duration(start))

    with httpcore.ConnectionPool(max_connections=POOL_LIMIT) as pool:
        # warmup
        with ThreadPoolExecutor(max_workers=CONCURRENCY * 2) as exec:
            run_in_executor(
                (lambda: httpcore_get(pool, []) for _ in range(REQUESTS)),
                exec,
            )

        timings: List[int] = []
        exec = ThreadPoolExecutor(max_workers=CONCURRENCY)
        start = time.monotonic()
        with profile():
            for _ in range(REPEATS):
                run_in_executor(
                    (lambda: httpcore_get(pool, timings) for _ in range(REQUESTS)), exec
                )
        exec.shutdown(wait=True)
        axis.plot(
            [*range(len(timings))], timings, label=f"httpcore (tot={duration(start)}ms)"
        )

    with urllib3.HTTPConnectionPool(
            "localhost", PORT, maxsize=POOL_LIMIT
    ) as urllib3_pool:
        # warmup
        with ThreadPoolExecutor(max_workers=CONCURRENCY * 2) as exec:
            run_in_executor(
                (lambda: urllib3_get(urllib3_pool, []) for _ in range(REQUESTS)),
                exec,
            )

        timings = []
        exec = ThreadPoolExecutor(max_workers=CONCURRENCY)
        start = time.monotonic()
        for _ in range(REPEATS):
            run_in_executor(
                (lambda: urllib3_get(urllib3_pool, timings) for _ in range(REQUESTS)),
                exec,
            )
        exec.shutdown(wait=True)
        axis.plot(
            [*range(len(timings))], timings, label=f"urllib3 (tot={duration(start)}ms)"
        )


def main() -> None:
    mode = sys.argv[1] if len(sys.argv) == 2 else None
    assert mode in ("async", "sync"), "Usage: python client.py <async|sync>"

    fig, ax = plt.subplots()

    if mode == "async":
        asyncio.run(run_async_requests(ax))
    else:
        run_sync_requests(ax)

    plt.legend(loc="upper left")
    ax.set_xlabel("# request")
    ax.set_ylabel("[ms]")
    plt.show()
    print("DONE", flush=True)


if __name__ == "__main__":
    main()