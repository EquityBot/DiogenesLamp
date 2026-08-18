import threading
from pathlib import Path

import pytest

from lamp.http import BlockedError, Client, FetchError, RateLimiter


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
    # The win from decoupling the global and host reservations: host a's
    # second call reserves a 1.0s host-gap wait but must NOT push
    # _last_global out to that same 1.0s mark - otherwise every other host
    # gets throttled below the global cap while host a sleeps out its own
    # gap. Host b's call, issued while host a's second call is genuinely
    # still asleep, must see only two global-gap increments (0.5s), not the
    # old buggy 1.25s (host a's 1.0s ready + one more global gap).
    clock = GatedClock()
    limiter = RateLimiter(global_rps=4.0, host_rps=1.0, time_fn=clock.time, sleep_fn=clock.sleep)
    limiter.wait("a")  # t=0, first request free, no sleep

    clock.gate_next_sleep = True
    worker = threading.Thread(target=limiter.wait, args=("a",))
    worker.start()
    assert clock.reserved.wait(timeout=5), "worker never reached its sleep"

    # Host a's second call already reserved its slot (computed a 1.0s sleep)
    # but is parked before actually sleeping - the clock has not moved.
    # Host b's reservation happens concurrently with host a's sleep.
    limiter.wait("b")
    assert clock.now == pytest.approx(0.5)
    assert clock.sleeps == pytest.approx([0.5])

    clock.release.set()
    worker.join(timeout=5)
    assert not worker.is_alive()
    assert clock.now == pytest.approx(1.5)
    assert clock.sleeps == pytest.approx([0.5, 1.0])


class FakeResponse:
    def __init__(self, status_code: int, content: bytes = b"ok"):
        self.status_code = status_code
        self.content = content
        self.text = content.decode()

    def iter_content(self, chunk_size: int):
        yield self.content


class FakeSession:
    def __init__(self, responses):
        self.responses = list(responses)
        self.calls: list[dict] = []

    def get(self, url, headers=None, timeout=None):
        self.calls.append({"url": url, "headers": headers or {}})
        result = self.responses.pop(0)
        if isinstance(result, Exception):
            raise result
        return result


def _client(responses):
    clock = FakeClock()
    limiter = RateLimiter(time_fn=clock.time, sleep_fn=clock.sleep)
    session = FakeSession(responses)
    return Client(limiter, session=session, sleep_fn=clock.sleep), session


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
    client, _ = _client([FakeResponse(200, b"jpegbytes")])
    dest = tmp_path / "thumbs" / "s" / "1.jpg"
    assert client.download("https://photo.yupoo.com/x/small.jpg", dest) is True
    assert dest.read_bytes() == b"jpegbytes"
    assert list(dest.parent.iterdir()) == [dest]


def test_download_failure_returns_false(tmp_path: Path):
    client, _ = _client([FakeResponse(404)])
    assert client.download("https://photo.yupoo.com/x.jpg", tmp_path / "t.jpg") is False
