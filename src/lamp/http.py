"""Polite HTTP client: global + per-host rate limits, retries, atomic downloads."""

from __future__ import annotations

import os
import threading
import time
from pathlib import Path
from urllib.parse import urlparse

import requests

USER_AGENT = "DiogenesLamp/0.1 (+https://github.com/ryanduguid/DiogenesLamp)"
_RETRIES = 3
_BACKOFF = [1.0, 2.0, 4.0]
_TIMEOUT = 30


class FetchError(Exception):
    pass


class BlockedError(FetchError):
    def __init__(self, status: int):
        super().__init__(f"blocked with HTTP {status}")
        self.status = status


class RateLimiter:
    def __init__(self, global_rps: float = 4.0, host_rps: float = 1.0,
                 host_overrides: dict[str, float] | None = None,
                 time_fn=time.monotonic, sleep_fn=time.sleep):
        self._global_gap = 1.0 / global_rps
        self._host_gap = 1.0 / host_rps
        self._host_override_gaps = {
            host: 1.0 / rps for host, rps in (host_overrides or {}).items()
        }
        self._time = time_fn
        self._sleep = sleep_fn
        self._lock = threading.Lock()
        self._last_global = float("-inf")
        self._last_host: dict[str, float] = {}

    def wait(self, host: str) -> None:
        # Reserve the slot under the lock, sleep outside it: a long host gap
        # for one host must not stall workers waiting on other hosts. The
        # global slot and the host slot are reserved independently - a host
        # gap that pushes this call's own ready time out must not also drag
        # _last_global with it, or every OTHER host gets throttled below the
        # global cap while this one host is waiting out its own gap.
        with self._lock:
            now = self._time()
            host_gap = self._host_override_gaps.get(host, self._host_gap)
            global_slot = max(now, self._last_global + self._global_gap)
            self._last_global = global_slot
            host_ready = self._last_host.get(host, float("-inf")) + host_gap
            ready = max(global_slot, host_ready)
            self._last_host[host] = ready
        delay = ready - now
        if delay > 0:
            self._sleep(delay)


class Client:
    def __init__(self, limiter: RateLimiter, user_agent: str = USER_AGENT,
                 session: requests.Session | None = None, sleep_fn=time.sleep):
        self._limiter = limiter
        self._session = session or requests.Session()
        self._headers = {"User-Agent": user_agent}
        self._sleep = sleep_fn

    def get(self, url: str, referer: str | None = None) -> requests.Response:
        headers = dict(self._headers)
        if referer:
            headers["Referer"] = referer
        host = urlparse(url).netloc
        last_error: Exception | None = None
        for attempt in range(_RETRIES):
            self._limiter.wait(host)
            try:
                response = self._session.get(url, headers=headers, timeout=_TIMEOUT)
            except requests.RequestException as exc:
                last_error = exc
                self._sleep(_BACKOFF[attempt])
                continue
            if response.status_code in (429, 403):
                raise BlockedError(response.status_code)
            if 500 <= response.status_code < 600:
                last_error = FetchError(f"HTTP {response.status_code} for {url}")
                self._sleep(_BACKOFF[attempt])
                continue
            if response.status_code != 200:
                raise FetchError(f"HTTP {response.status_code} for {url}")
            return response
        raise FetchError(f"gave up on {url}: {last_error}")

    def download(self, url: str, dest: Path, referer: str | None = None) -> bool:
        try:
            response = self.get(url, referer=referer)
        except BlockedError:
            raise
        except FetchError:
            return False
        dest.parent.mkdir(parents=True, exist_ok=True)
        tmp = dest.with_name(dest.name + ".tmp")
        with open(tmp, "wb") as fh:
            fh.writelines(response.iter_content(chunk_size=65536))
        os.replace(tmp, dest)
        return True
