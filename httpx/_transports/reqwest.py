import ssl
from collections.abc import AsyncIterable, AsyncIterator, Generator
from contextlib import contextmanager
from datetime import timedelta
from types import TracebackType

from multidict import CIMultiDict
from typing_extensions import Self

from .._config import DEFAULT_LIMITS, Limits, Timeout, Proxy
from .._content import ByteStream
from .._exceptions import (
    ConnectError as HttpxConnectError,
    PoolTimeout as HttpxPoolTimeout,
    ReadError as HttpxReadError,
    ReadTimeout as HttpxReadTimeout,
    UnsupportedProtocol as HttpxUnsupportedProtocol,
    WriteTimeout as HttpxWriteTimeout,
)
from .._types import AsyncByteStream, Middleware
from . import AsyncBaseTransport

import rustimport.import_hook  # noqa:F401
from .reqwest_native import (
    BadUrlError,
    NativeAsyncClient,
    NativeProxyConfig,
    PoolTimeoutError,
    ReadConnectionError,
    ReadTimeoutError,
    SendConnectionError,
    SendTimeoutError,
)


class AsyncReqwestHTTPTransport(AsyncBaseTransport):
    def __init__(
        self,
        http1: bool = True,
        http2: bool = False,
        timeout: Timeout | None = None,
        limits: Limits = DEFAULT_LIMITS,
        ssl_context: ssl.SSLContext | None = None,
        proxy: Proxy | None = None,
        middlewares: list[Middleware] = None,
    ) -> None:
        self._client = NativeAsyncClient(
            total_timeout=self._total_timeout(timeout),
            connect_timeout=self._connect_timeout(timeout),
            read_timeout=timedelta(seconds=timeout.read) if timeout and timeout.read else None,
            pool_idle_timeout=timedelta(seconds=limits.keepalive_expiry) if limits.keepalive_expiry else None,
            pool_max_idle_per_host=limits.max_keepalive_connections,
            max_connections=limits.max_connections,
            http1=http1,
            http2=http2,
            root_certificates_der=ssl_context.get_ca_certs(binary_form=True) if ssl_context else None,
            proxy=self._proxy_config(proxy),
            middlewares=middlewares,
        )

    def _proxy_config(self, proxy: Proxy | None) -> NativeProxyConfig | None:
        if proxy is None:
            return None
        return NativeProxyConfig(
            url=str(proxy.url),
            basic_auth=proxy.raw_auth,
            headers=proxy.headers,
        )

    def _total_timeout(self, timeout: Timeout | None) -> timedelta | None:
        # Workaround for https://github.com/seanmonstar/reqwest/issues/2403
        if timeout is None:
            return None
        if not timeout.write:
            return None
        return timedelta(
            seconds=timeout.write + (timeout.connect or 0.0) + (timeout.read or 0.0) + (timeout.pool or 0.0),
        )

    def _connect_timeout(self, timeout: Timeout | None) -> timedelta | None:
        if timeout is None:
            return None
        if not (timeout.connect or timeout.pool):
            return None
        return timedelta(seconds=(timeout.connect or 0.0) + (timeout.pool or 0.0))

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None = None,
        exc_value: BaseException | None = None,
        traceback: TracebackType | None = None,
    ) -> None:
        await self.aclose()

    async def handle_async_request(self, method: str, url: str) -> "Response":
        # body, stream = None, None
        # if isinstance(request.stream, ByteStream):
        #     body = request.stream.content
        # else:
        #     assert isinstance(request.stream, AsyncByteStream)
        #     stream = aiter(request.stream)

        with _map_errors():
            # resp = await self._client.request(
            #     method=request.method,
            #     url=str(request.url),
            #     headers=request.headers,
            #     body=body,
            #     stream=stream,
            #     timeout=None,
            #     extensions=request.extensions,
            # )
            resp = await self._client.request(
                method=method,
                url=url,
                headers={},
                body=b"",
                stream=None,
                timeout=None,
                extensions={},
            )

        return Response(resp)

    async def aclose(self) -> None:
        await self._client.close()


class Response:
    def __init__(
        self,
        response,
    ) -> None:
        self.status_code = response.head.status_code
        self.headers = response.head.headers
        self.chunks = None
        self.stream = None
        if isinstance(response.body, list):
            self.chunks = response.body
        else:
            self.stream = AsyncResponseStream(response.body)
        self.extensions = {}

    async def read(self) -> memoryview | bytes:
        if self.chunks is not None:
            return b"".join([c for c in self.chunks])
        res = [c async for chunks in self.stream for c in chunks]
        return b"".join(res)

    async def aclose(self) -> None:
        if self.stream is not None:
            await self.stream.aclose()


class AsyncResponseStream(AsyncByteStream):
    def __init__(self, response_stream: AsyncIterable[memoryview]) -> None:
        self.stream = response_stream

    async def __aiter__(self) -> AsyncIterator[list[memoryview]]:
        with _map_errors():
            has_more = True
            while has_more:
                b, has_more = self.stream.try_next_no_wait()
                if b is None and has_more:
                    b, has_more = await self.stream.wait_next()
                if b is not None:
                    yield b

    async def aclose(self) -> None:
        if hasattr(self.stream, "close"):
            await self.stream.close()


@contextmanager
def _map_errors() -> Generator[None, None, None]:
    try:
        yield
    except BadUrlError as e:
        raise HttpxUnsupportedProtocol(str(e)) from e
    except SendConnectionError as e:
        raise HttpxConnectError(str(e)) from e
    except SendTimeoutError as e:
        raise HttpxWriteTimeout(str(e)) from e
    except PoolTimeoutError as e:
        raise HttpxPoolTimeout(str(e)) from e
    except ReadConnectionError as e:
        raise HttpxReadError(str(e)) from e
    except ReadTimeoutError as e:
        raise HttpxReadTimeout(str(e)) from e


def _supports_buffer_protocol(obj):
    try:
        memoryview(obj)
        return True
    except TypeError:
        return False
