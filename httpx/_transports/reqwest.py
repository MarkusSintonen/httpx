import ssl
from collections.abc import AsyncIterable, AsyncIterator, Generator
from contextlib import contextmanager
from datetime import timedelta
from types import TracebackType

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
from .._models import Request, Response
from .._types import AsyncByteStream, Tracer
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
        tracer: Tracer = None,
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
            tracer=tracer,
        )

    def _proxy_config(self, proxy: Proxy | None) -> NativeProxyConfig | None:
        if proxy is None:
            return None
        return NativeProxyConfig(
            url=str(proxy.url),
            basic_auth=proxy.raw_auth,
            headers=proxy.headers.raw,
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

    async def handle_async_request(self, request: Request) -> Response:
        if isinstance(request.stream, ByteStream):
            bytes_iter = iter(request.stream)
            body_bytes = next(bytes_iter)
            assert next(bytes_iter, None) is None
        else:
            assert isinstance(request.stream, AsyncByteStream)
            bytes_iter = aiter(request.stream)
            body_bytes = None

        with _map_errors():
            resp = await self._client.request(
                method=request.method,
                url=str(request.url),
                headers=request.headers.raw,
                content=body_bytes if body_bytes is not None else bytes_iter,
                timeout=None,
                extensions=request.extensions,
            )

        return Response(
            status_code=resp.status,
            headers=resp.headers,
            stream=AsyncResponseStream(resp),
            extensions=await resp.get_extensions(),
        )

    async def aclose(self) -> None:
        await self._client.close()


class AsyncResponseStream(AsyncByteStream):
    def __init__(self, response_stream: AsyncIterable[bytes]) -> None:
        self._response_stream = response_stream

    async def __aiter__(self) -> AsyncIterator[bytes]:
        with _map_errors():
            async for part in self._response_stream:
                yield part

    async def aclose(self) -> None:
        if hasattr(self._response_stream, "close"):
            await self._response_stream.close()


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
