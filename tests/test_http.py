import threading
from io import BytesIO
from pathlib import Path

import pytest
from PIL import Image

from lamp.http import _RETRIES, BlockedError, Client, FetchError, RateLimiter


class FakeClock:
    def __init__(self):
        self.now = 0.0
        self.sleeps: list[float] = []

    def time(self) -> float:
        return self.now

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.now += seconds


class GatedClock(FakeClock):
    """FakeClock where one designated sleep() call blocks before it advances
    the clock, until the test releases it - standing in for a real worker
    thread that is genuinely asleep (wall-clock barely moved) while another
    thread's wait() reserves its own slot under the limiter's lock."""

    def __init__(self):
        super().__init__()
        self.gate_next_sleep = False
        self.reserved = threading.Event()
        self.release = threading.Event()

    def sleep(self, seconds: float) -> None:
        if self.gate_next_sleep:
            self.gate_next_sleep = False
            self.reserved.set()
            assert self.release.wait(timeout=5), "test deadlocked waiting for release"
        super().sleep(seconds)


def test_rate_limiter_enforces_global_and_host_gaps():
    clock = FakeClock()
    limiter = RateLimiter(global_rps=4.0, host_rps=1.0, time_fn=clock.time, sleep_fn=clock.sleep)
    limiter.wait("a.example")          # t=0.00 first request free
    limiter.wait("b.example")          # global gap 0.25s
    assert clock.now == pytest.approx(0.25)
    limiter.wait("a.example")          # host gap: 1s after a.example's last
    assert clock.now == pytest.approx(1.0)


def test_rate_limiter_host_override_uses_global_gap():
    clock = FakeClock()
    limiter = RateLimiter(global_rps=4.0, host_rps=1.0,
                          host_overrides={"photo.example": 4.0},
                          time_fn=clock.time, sleep_fn=clock.sleep)
    limiter.wait("photo.example")
    limiter.wait("photo.example")      # override: 0.25s gap, not 1s
    assert clock.now == pytest.approx(0.25)


def test_rate_limiter_host_delay_does_not_drag_back_global_slot():
    # Host b can use the next global slot while host a waits for its own gap.
    clock = GatedClock()
    limiter = RateLimiter(global_rps=4.0, host_rps=1.0, time_fn=clock.time, sleep_fn=clock.sleep)
    limiter.wait("a")  # t=0, first request free, no sleep

    clock.gate_next_sleep = True
    worker = threading.Thread(target=limiter.wait, args=("a",))
    worker.start()
    assert clock.reserved.wait(timeout=5), "worker never reached its sleep"

    try:
        limiter.wait("b")
        assert clock.now == pytest.approx(0.25)
        assert clock.sleeps == pytest.approx([0.25])
    finally:
        clock.release.set()
        worker.join(timeout=5)
    assert not worker.is_alive()
    assert clock.now == pytest.approx(1.25)
    assert clock.sleeps == pytest.approx([0.25, 1.0])


def test_get_retries_broken_body_and_preserves_decoded_response():
    import requests

    closed = []

    class BrokenResponse(requests.Response):
        def iter_content(self, chunk_size):
            yield b'partial'
            raise requests.exceptions.ChunkedEncodingError('broken stream')

        def close(self):
            closed.append(True)

    broken = BrokenResponse()
    broken.status_code = 200
    good = requests.Response()
    good.status_code = 200
    good.encoding = 'utf-8'
    good.raw = BytesIO('catalogue café'.encode())
    client, transport = _client([broken, good])
    response = client.get('https://example.test/')
    assert response.content == 'catalogue café'.encode()
    assert response.text == 'catalogue café'
    assert closed == [True]
    assert len(transport.calls) == 2


class FakeResponse:
    def __init__(self, status_code: int, content: bytes = b"ok", headers=None):
        self.status_code = status_code
        self.content = content
        self.headers = headers or {}
        self.text = content.decode(errors="replace")

    def iter_content(self, chunk_size: int):
        yield self.content

    def close(self):
        pass


def test_delayed_host_rechecks_global_gap_when_other_hosts_overtake():
    class Clock(GatedClock):
        def sleep(self, seconds):
            deadline = self.now + seconds
            if self.gate_next_sleep:
                self.gate_next_sleep = False
                self.reserved.set()
                assert self.release.wait(timeout=5)
            self.now = max(self.now, deadline)

    clock = Clock()
    limiter = RateLimiter(global_rps=4, host_rps=1, time_fn=clock.time, sleep_fn=clock.sleep)
    limiter.wait('a')
    clock.gate_next_sleep = True
    admitted = []
    worker = threading.Thread(target=lambda: (limiter.wait('a'), admitted.append(clock.now)))
    worker.start()
    try:
        assert clock.reserved.wait(timeout=5)
        clock.now = 1.0
        limiter.wait('b')
    finally:
        clock.release.set()
        worker.join(timeout=5)
    assert not worker.is_alive()
    assert admitted == [1.25]


def test_get_bounds_stream_and_closes_oversized_response(monkeypatch):
    import lamp.http as http
    monkeypatch.setattr(http, '_MAX_HTML_BYTES', 10, raising=False)
    closed = []

    class LargeResponse(FakeResponse):
        def iter_content(self, chunk_size):
            yield b'123456'
            yield b'789012'
            pytest.fail('read beyond limit')

        def close(self):
            closed.append(True)

    client, transport = _client([LargeResponse(200)])
    with pytest.raises(FetchError, match='HTML.*limit'):
        client.get('https://example.test/')
    assert closed == [True]
    assert len(transport.calls) == 1


class FakeTransport:
    def __init__(self, responses):
        self.responses = list(responses)
        self.calls: list[dict] = []

    def __call__(self, target, headers):
        self.calls.append({"url": target.url, "headers": headers or {}})
        result = self.responses.pop(0)
        if isinstance(result, Exception):
            raise result
        return result, getattr(result, "close", lambda: None)


def _client(responses):
    clock = FakeClock()
    limiter = RateLimiter(time_fn=clock.time, sleep_fn=clock.sleep)
    transport = FakeTransport(responses)
    return (
        Client(
            limiter,
            resolver=lambda _host, _port: ("8.8.8.8",),
            transport=transport,
            sleep_fn=clock.sleep,
        ),
        transport,
    )


def _png_bytes() -> bytes:
    buffer = BytesIO()
    Image.new("RGB", (2, 2), "red").save(buffer, format="PNG")
    return buffer.getvalue()


def test_get_sets_headers_and_referer():
    client, session = _client([FakeResponse(200)])
    client.get("https://sellerx.x.yupoo.com/albums?page=1", referer="https://sellerx.x.yupoo.com")
    headers = session.calls[0]["headers"]
    assert "DiogenesLamp" in headers["User-Agent"]
    assert headers["Referer"] == "https://sellerx.x.yupoo.com"


def test_get_retries_5xx_then_succeeds():
    client, session = _client([FakeResponse(500), FakeResponse(502), FakeResponse(200)])
    assert client.get("https://x.example/").status_code == 200
    assert len(session.calls) == 3


def test_get_raises_blocked_on_429_without_retry():
    client, session = _client([FakeResponse(429)])
    with pytest.raises(BlockedError):
        client.get("https://x.example/")
    assert len(session.calls) == 1


def test_get_gives_up_after_retries():
    import requests

    client, _ = _client([requests.ConnectionError(), requests.ConnectionError(),
                         requests.ConnectionError()])
    with pytest.raises(FetchError):
        client.get("https://x.example/")


def test_download_writes_file_atomically(tmp_path: Path):
    payload = _png_bytes()
    client, _ = _client([FakeResponse(200, payload, {"Content-Type": "image/png"})])
    dest = tmp_path / "thumbs" / "s" / "1.jpg"
    assert client.download("https://photo.yupoo.com/x/small.jpg", dest) is True
    assert dest.read_bytes() == payload
    assert list(dest.parent.iterdir()) == [dest]


def test_download_failure_returns_false(tmp_path: Path):
    client, _ = _client([FakeResponse(404)])
    assert client.download("https://photo.yupoo.com/x.jpg", tmp_path / "t.jpg") is False


def test_download_stream_break_returns_false_and_cleans_tmp(tmp_path: Path):
    """A connection that dies mid-body is retried as a fresh GET; after all
    retries the partial .tmp must not be left behind."""
    import requests

    class BrokenStream:
        status_code = 200
        headers = {"Content-Type": "image/png"}

        def iter_content(self, chunk_size):
            yield b"partial"
            raise requests.exceptions.ChunkedEncodingError("died mid-body")

    client, session = _client([BrokenStream(), BrokenStream(), BrokenStream()])
    dest = tmp_path / "thumbs" / "1.jpg"
    assert client.download("https://photo.yupoo.com/x.jpg", dest) is False
    assert not dest.exists()
    assert list(dest.parent.iterdir()) == []  # no .tmp left behind
    assert len(session.calls) == 3  # retried the whole GET, not just the stream


def test_download_stream_break_then_success(tmp_path: Path):
    """A flaky first attempt must not cost the thumb: the retry succeeds."""
    import requests

    class BrokenStream:
        status_code = 200
        headers = {"Content-Type": "image/png"}

        def iter_content(self, chunk_size):
            yield b"partial"
            raise requests.exceptions.ChunkedEncodingError("died mid-body")

    payload = _png_bytes()
    client, _ = _client(
        [BrokenStream(), FakeResponse(200, payload, {"Content-Type": "image/png"})]
    )
    dest = tmp_path / "1.jpg"
    assert client.download("https://photo.yupoo.com/x.jpg", dest) is True
    assert dest.read_bytes() == payload


def test_get_does_not_sleep_before_giving_up():
    """The last failed attempt used to sleep the full backoff (4s) before
    raising anyway - dead time per permanently failed fetch."""
    import requests

    clock = FakeClock()
    transport = FakeTransport([requests.ConnectionError()] * _RETRIES)
    client = Client(
        RateLimiter(time_fn=clock.time, sleep_fn=clock.sleep),
        resolver=lambda _host, _port: ("8.8.8.8",),
        transport=transport,
        sleep_fn=clock.sleep,
    )
    with pytest.raises(FetchError):
        client.get("https://x.example/")
    assert clock.sleeps == [1.0, 2.0]  # doubling, and no sleep after the final attempt
