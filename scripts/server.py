import asyncio

import uvicorn

PORT = 1234
RESP = b"a" * 2000000
SLEEP = 0


async def app(scope, receive, send):
    assert scope["type"] == "http"
    assert scope["path"] == "/req"
    assert not (await receive()).get("more_body", False)

    await asyncio.sleep(SLEEP)
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [[b"content-type", b"text/plain"], [b"content-length", str(len(RESP)).encode()]],
        }
    )
    await send(
        {
            "type": "http.response.body",
            "body": RESP,
        }
    )


if __name__ == "__main__":
    uvicorn.run(
        app,
        port=PORT,
        log_level="info",
        access_log=False,
        # Keep warmed up connections alive during the test to have consistent results across test runs.
        # This avoids timing differences with connections getting closed and reopened in the background.
        timeout_keep_alive=100,
    )