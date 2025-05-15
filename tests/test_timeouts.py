import asyncio

import pytest

import httpx


async def test_read_timeout(server):
    timeout = httpx.Timeout(None, read=0.1)

    async with httpx.AsyncClient(timeout=timeout) as client:
        with pytest.raises(httpx.ReadTimeout):
            await client.get(server.url.copy_with(path="/slow_response"))


async def test_write_timeout(server):
    timeout = httpx.Timeout(None, write=1e-6)

    async with httpx.AsyncClient(timeout=timeout) as client:
        with pytest.raises(httpx.WriteTimeout):
            data = b"*" * 1024 * 1024 * 100
            await client.put(server.url.copy_with(path="/slow_response"), content=data)


@pytest.mark.network
async def test_connect_timeout(server):
    timeout = httpx.Timeout(None, connect=1e-6)

    async with httpx.AsyncClient(timeout=timeout) as client:
        with pytest.raises((httpx.ConnectTimeout, httpx.ConnectError)):
            # See https://stackoverflow.com/questions/100841/
            await client.get("http://10.255.255.1/")


async def test_pool_timeout(server):
    limits = httpx.Limits(max_connections=1)
    timeout = httpx.Timeout(None, pool=0.1)

    async with httpx.AsyncClient(limits=limits, timeout=timeout) as client:
        with pytest.raises(httpx.PoolTimeout):
            await asyncio.gather(
                client.get(server.url.copy_with(path="/slow_response")),
                client.get(server.url.copy_with(path="/slow_response")),
            )


async def test_async_client_new_request_send_timeout(server):
    timeout = httpx.Timeout(1e-6)

    async with httpx.AsyncClient(timeout=timeout) as client:
        with pytest.raises(httpx.TimeoutException):
            await client.send(
                httpx.Request("GET", server.url.copy_with(path="/slow_response")),
            )
