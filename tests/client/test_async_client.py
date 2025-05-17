from __future__ import annotations

import ssl
import typing
from dataclasses import dataclass
from datetime import timedelta

import pytest

import httpx



async def test_get(server):
    url = server.url
    async with httpx.AsyncClient(http2=True) as client:
        response = await client.get(url)
    assert response.status_code == 200
    assert response.text == "Hello, world!"
    assert response.http_version == "HTTP/1.1"
    assert response.url.scheme == "http"
    assert response.headers
    assert repr(response) == "<Response [200 OK]>"
    assert response.elapsed > timedelta(seconds=0)


async def test_get_https(https_server, client_pem_file):
    ssl_context = ssl.create_default_context(cafile=client_pem_file)
    async with httpx.AsyncClient(http2=True, verify=ssl_context) as client:
        response = await client.get(https_server.url)
    assert response.status_code == 200
    assert response.text == "Hello, world!"
    assert response.http_version == "HTTP/1.1"
    assert response.url.scheme == "https"
    assert response.headers
    assert repr(response) == "<Response [200 OK]>"
    assert response.elapsed > timedelta(seconds=0)


async def test_get_https__fails_not_trusted(https_server):
    ssl_context = ssl.create_default_context()
    async with httpx.AsyncClient(http2=True, verify=ssl_context) as client:
        with pytest.raises(httpx.ConnectError):
            await client.get(https_server.url)


@pytest.mark.parametrize(
    "url",
    [
        pytest.param("invalid://example.org", id="scheme-not-http(s)"),
        pytest.param("://example.org", id="no-scheme"),
        pytest.param("http://", id="no-host"),
    ],
)

async def test_get_invalid_url(server, url):
    async with httpx.AsyncClient() as client:
        with pytest.raises((httpx.UnsupportedProtocol, httpx.LocalProtocolError)):
            await client.get(url)



async def test_build_request(server):
    url = server.url.copy_with(path="/echo_headers")
    headers = {"Custom-header": "value"}
    async with httpx.AsyncClient() as client:
        request = client.build_request("GET", url)
        request.headers.update(headers)
        response = await client.send(request)

    assert response.status_code == 200
    assert response.url == url

    assert response.json()["Custom-header"] == "value"



async def test_post(server):
    url = server.url
    async with httpx.AsyncClient() as client:
        response = await client.post(url, content=b"Hello, world!")
    assert response.status_code == 200



async def test_post_json(server):
    url = server.url
    async with httpx.AsyncClient() as client:
        response = await client.post(url, json={"text": "Hello, world!"})
    assert response.status_code == 200



async def test_stream_response(server):
    async with httpx.AsyncClient() as client:
        async with client.stream("GET", server.url) as response:
            body = await response.aread()

    assert response.status_code == 200
    assert body == b"Hello, world!"
    assert response.content == b"Hello, world!"



async def test_access_content_stream_response(server):
    async with httpx.AsyncClient() as client:
        async with client.stream("GET", server.url) as response:
            pass

    assert response.status_code == 200
    with pytest.raises(httpx.ResponseNotRead):
        response.content  # noqa: B018



async def test_stream_request(server):
    async def hello_world() -> typing.AsyncIterator[bytes]:
        yield b"Hello, "
        yield b"world!"

    async with httpx.AsyncClient() as client:
        response = await client.post(server.url, content=hello_world())
    assert response.status_code == 200



async def test_cannot_stream_sync_request(server):
    def hello_world() -> typing.Iterator[bytes]:  # pragma: no cover
        yield b"Hello, "
        yield b"world!"

    async with httpx.AsyncClient() as client:
        with pytest.raises(RuntimeError):
            await client.post(server.url, content=hello_world())



async def test_raise_for_status(server):
    async with httpx.AsyncClient() as client:
        for status_code in (200, 400, 404, 500, 505):
            response = await client.request(
                "GET", server.url.copy_with(path=f"/status/{status_code}")
            )

            if 400 <= status_code < 600:
                with pytest.raises(httpx.HTTPStatusError) as exc_info:
                    response.raise_for_status()
                assert exc_info.value.response == response
            else:
                assert response.raise_for_status() is response



async def test_options(server):
    async with httpx.AsyncClient() as client:
        response = await client.options(server.url)
    assert response.status_code == 200
    assert response.text == "Hello, world!"



async def test_head(server):
    async with httpx.AsyncClient() as client:
        response = await client.head(server.url)
    assert response.status_code == 200
    assert response.text == ""



async def test_put(server):
    async with httpx.AsyncClient() as client:
        response = await client.put(server.url, content=b"Hello, world!")
    assert response.status_code == 200



async def test_patch(server):
    async with httpx.AsyncClient() as client:
        response = await client.patch(server.url, content=b"Hello, world!")
    assert response.status_code == 200



async def test_delete(server):
    async with httpx.AsyncClient() as client:
        response = await client.delete(server.url)
    assert response.status_code == 200
    assert response.text == "Hello, world!"



async def test_100_continue(server):
    headers = {"Expect": "100-continue"}
    content = b"Echo request body"

    async with httpx.AsyncClient() as client:
        response = await client.post(
            server.url.copy_with(path="/echo_body"), headers=headers, content=content
        )

    assert response.status_code == 200
    assert response.content == content



async def test_context_managed_transport():
    class Transport(httpx.AsyncBaseTransport):
        def __init__(self) -> None:
            self.events: list[str] = []

        async def aclose(self):
            # The base implementation of httpx.AsyncBaseTransport just
            # calls into `.aclose`, so simple transport cases can just override
            # this method for any cleanup, where more complex cases
            # might want to additionally override `__aenter__`/`__aexit__`.
            self.events.append("transport.aclose")

        async def __aenter__(self):
            await super().__aenter__()
            self.events.append("transport.__aenter__")

        async def __aexit__(self, *args):
            await super().__aexit__(*args)
            self.events.append("transport.__aexit__")

    transport = Transport()
    async with httpx.AsyncClient(transport=transport):
        pass

    assert transport.events == [
        "transport.__aenter__",
        "transport.aclose",
        "transport.__aexit__",
    ]



async def test_context_managed_transport_and_mount():
    class Transport(httpx.AsyncBaseTransport):
        def __init__(self, name: str) -> None:
            self.name: str = name
            self.events: list[str] = []

        async def aclose(self):
            # The base implementation of httpx.AsyncBaseTransport just
            # calls into `.aclose`, so simple transport cases can just override
            # this method for any cleanup, where more complex cases
            # might want to additionally override `__aenter__`/`__aexit__`.
            self.events.append(f"{self.name}.aclose")

        async def __aenter__(self):
            await super().__aenter__()
            self.events.append(f"{self.name}.__aenter__")

        async def __aexit__(self, *args):
            await super().__aexit__(*args)
            self.events.append(f"{self.name}.__aexit__")

    transport = Transport(name="transport")
    mounted = Transport(name="mounted")
    async with httpx.AsyncClient(
        transport=transport, mounts={"http://www.example.org": mounted}
    ):
        pass

    assert transport.events == [
        "transport.__aenter__",
        "transport.aclose",
        "transport.__aexit__",
    ]
    assert mounted.events == [
        "mounted.__aenter__",
        "mounted.aclose",
        "mounted.__aexit__",
    ]


def hello_world(request):
    return httpx.Response(200, text="Hello, world!")



async def test_client_closed_state_using_implicit_open():
    client = httpx.AsyncClient(transport=httpx.MockTransport(hello_world))

    assert not client.is_closed
    await client.get("http://example.com")

    assert not client.is_closed
    await client.aclose()

    assert client.is_closed
    # Once we're close we cannot make any more requests.
    with pytest.raises(RuntimeError):
        await client.get("http://example.com")

    # Once we're closed we cannot reopen the client.
    with pytest.raises(RuntimeError):
        async with client:
            pass  # pragma: no cover



async def test_client_closed_state_using_with_block():
    async with httpx.AsyncClient(transport=httpx.MockTransport(hello_world)) as client:
        assert not client.is_closed
        await client.get("http://example.com")

    assert client.is_closed
    with pytest.raises(RuntimeError):
        await client.get("http://example.com")


def unmounted(request: httpx.Request) -> httpx.Response:
    data = {"app": "unmounted"}
    return httpx.Response(200, json=data)


def mounted(request: httpx.Request) -> httpx.Response:
    data = {"app": "mounted"}
    return httpx.Response(200, json=data)



async def test_mounted_transport():
    transport = httpx.MockTransport(unmounted)
    mounts = {"custom://": httpx.MockTransport(mounted)}

    async with httpx.AsyncClient(transport=transport, mounts=mounts) as client:
        response = await client.get("https://www.example.com")
        assert response.status_code == 200
        assert response.json() == {"app": "unmounted"}

        response = await client.get("custom://www.example.com")
        assert response.status_code == 200
        assert response.json() == {"app": "mounted"}



async def test_async_mock_transport():
    async def hello_world(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, text="Hello, world!")

    transport = httpx.MockTransport(hello_world)

    async with httpx.AsyncClient(transport=transport) as client:
        response = await client.get("https://www.example.com")
        assert response.status_code == 200
        assert response.text == "Hello, world!"



async def test_cancellation_during_stream():
    """
    If any BaseException is raised during streaming the response, then the
    stream should be closed.

    This includes:

    * `asyncio.CancelledError` (A subclass of BaseException from Python 3.8 onwards.)
    * `trio.Cancelled`
    * `KeyboardInterrupt`
    * `SystemExit`

    See https://github.com/encode/httpx/issues/2139
    """
    stream_was_closed = False

    def response_with_cancel_during_stream(request):
        class CancelledStream(httpx.AsyncByteStream):
            async def __aiter__(self) -> typing.AsyncIterator[bytes]:
                yield b"Hello"
                raise KeyboardInterrupt()
                yield b", world"  # pragma: no cover

            async def aclose(self) -> None:
                nonlocal stream_was_closed
                stream_was_closed = True

        return httpx.Response(
            200, headers={"Content-Length": "12"}, stream=CancelledStream()
        )

    transport = httpx.MockTransport(response_with_cancel_during_stream)

    async with httpx.AsyncClient(transport=transport) as client:
        with pytest.raises(KeyboardInterrupt):
            await client.get("https://www.example.com")
        assert stream_was_closed



async def test_server_extensions(server):
    url = server.url
    async with httpx.AsyncClient(http2=True) as client:
        response = await client.get(url, extensions={"something_custom": "foo"})
    assert response.status_code == 200
    assert response.extensions["http_version"] == b"HTTP/1.1"
    assert response.extensions["something_custom"] == "foo"


@pytest.mark.parametrize("extra", [1, "a", [1, 2], ["a", "b"], {"a": "b"}, b"123"])
async def test_trace(server, extra):
    url = server.url

    @dataclass
    class Tracer:
        async def on_request_start(self, request_trace) -> None:
            assert request_trace.request.method == "GET"
            assert request_trace.request.url == str(url)
            assert request_trace.extensions["something_custom"] == "foo"
            request_trace.extensions["something_custom"] = "bar"
            request_trace.extensions["extra"] = extra

        async def on_request_end(self, response_trace) -> None:
            assert response_trace.request.method == "GET"
            assert response_trace.request.url == str(url)
            assert response_trace.response.status_code == 200
            assert response_trace.extensions["something_custom"] == "bar"
            assert response_trace.extensions["extra"] == extra
            response_trace.extensions["something_custom"] = "baz"

    async with httpx.AsyncClient(http2=True, tracer=Tracer()) as client:
        response = await client.get(url, extensions={"something_custom": "foo"})
    assert response.status_code == 200
    assert response.extensions["http_version"] == b"HTTP/1.1"
    assert response.extensions["something_custom"] == "baz"
    assert response.extensions["extra"] == extra


@pytest.mark.parametrize("exc_request", [Exception("some_req_error"), None])
@pytest.mark.parametrize("exc_response", [Exception("some_resp_error"), None])
async def test_trace_exception(server, exc_request: Exception | None, exc_response: Exception | None):
    @dataclass
    class Tracer:
        async def on_request_start(self, request_trace) -> None:
            if exc_request:
                raise exc_request

        async def on_request_end(self, response_trace) -> None:
            if exc_response:
                raise exc_response

    async with httpx.AsyncClient(http2=True, tracer=Tracer()) as client:
        if exc_request or exc_response:
            with pytest.raises(Exception) as e:
                await client.get(server.url, extensions={"something_custom": "foo"})
            assert e.value.__class__.__name__ == "SendUnknownError"
            assert str(exc_request or exc_response) in str(e.value)
        else:
            response = await client.get(server.url, extensions={"something_custom": "foo"})
            assert response.status_code == 200
            assert response.extensions["something_custom"] == "foo"
