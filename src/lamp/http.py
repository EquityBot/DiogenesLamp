"""Polite, rebinding-safe HTTP client with bounded atomic image downloads."""

from __future__ import annotations

import ipaddress
import os
import socket
import threading
import time
import warnings
from collections import OrderedDict
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urljoin, urlsplit, urlunsplit

import requests
from PIL import Image, UnidentifiedImageError
from requests.adapters import HTTPAdapter

USER_AGENT = "DiogenesLamp/0.1 (+https://github.com/ryanduguid/DiogenesLamp)"
_RETRIES = 3
_TIMEOUT = 30
_MAX_REDIRECTS = 5
_MAX_IMAGE_BYTES = 8 * 1024 * 1024
_MAX_HTML_BYTES = 8 * 1024 * 1024
_MAX_IMAGE_DIMENSION = 8192
_MAX_IMAGE_PIXELS = 20_000_000
_REDIRECT_STATUSES = frozenset({301, 302, 303, 307, 308})
_IMAGE_MIMES = {
    "JPEG": frozenset({"image/jpeg", "image/jpg"}),
    "PNG": frozenset({"image/png"}),
    "WEBP": frozenset({"image/webp"}),
}
_NAT64_WELL_KNOWN = ipaddress.ip_network("64:ff9b::/96")
_NAT64_LOCAL_USE = ipaddress.ip_network("64:ff9b:1::/48")


class FetchError(Exception):
    pass


class BlockedError(FetchError):
    def __init__(self, status: int):
        super().__init__(f"blocked with HTTP {status}")
        self.status = status


class _RetryableFetchError(FetchError):
    pass


class _InvalidImageError(FetchError):
    pass


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
        # Admit against the clock after every sleep. Other hosts can proceed
        # while this host waits, so a reserved global slot may no longer fit.
        while True:
            with self._lock:
                now = self._time()
                host_gap = self._host_override_gaps.get(host, self._host_gap)
                ready = max(self._last_global + self._global_gap,
                            self._last_host.get(host, float("-inf")) + host_gap)
                delay = ready - now
                if delay <= 0:
                    self._last_global = now
                    self._last_host[host] = now
                    return
            self._sleep(delay)


@dataclass(frozen=True)
class _Target:
    url: str
    hostname: str
    # Every validated address, in resolution order. One unreachable address must
    # not fail a host that has another, and each is pinned the same way.
    connect_ips: tuple[str, ...]
    port: int


Resolver = Callable[[str, int], Iterable[str]]
Transport = Callable[
    [_Target, Mapping[str, str]],
    tuple[requests.Response, Callable[[], None]],
]


def _host_header(hostname: str) -> str:
    try:
        address = ipaddress.ip_address(hostname)
    except ValueError:
        return hostname
    return f"[{hostname}]" if address.version == 6 else hostname


def _normalise_url(url: str) -> tuple[str, str, int]:
    if not isinstance(url, str) or not url:
        raise FetchError("refusing empty URL")
    if "\\" in url or any(ord(char) < 32 or ord(char) == 127 for char in url):
        raise FetchError(f"refusing malformed URL: {url}")

    # urlsplit raises ValueError on a malformed authority, an unmatched IPv6
    # bracket among them. download() contracts on FetchError, so a ValueError
    # escaping here aborts the scrape instead of failing the one URL.
    try:
        parts = urlsplit(url)
    except ValueError as exc:
        raise FetchError(f"refusing malformed URL: {url}") from exc
    if parts.scheme.lower() != "https":
        raise FetchError(f"refusing non-https URL: {url}")
    if parts.username is not None or parts.password is not None:
        raise FetchError(f"refusing URL with userinfo: {url}")
    try:
        port = parts.port or 443
    except ValueError as exc:
        raise FetchError(f"refusing invalid port in URL: {url}") from exc
    if port != 443:
        raise FetchError(f"refusing non-default HTTPS port: {url}")

    raw_hostname = parts.hostname
    if not raw_hostname or "%" in raw_hostname:
        raise FetchError(f"refusing invalid host: {url}")
    try:
        literal = ipaddress.ip_address(raw_hostname)
    except ValueError:
        try:
            hostname = raw_hostname.rstrip(".").encode("idna").decode("ascii").lower()
        except UnicodeError as exc:
            raise FetchError(f"refusing invalid host: {url}") from exc
    else:
        hostname = str(literal)
    if not hostname or hostname == "localhost" or hostname.endswith(".local"):
        raise FetchError(f"refusing local host: {url}")

    normalised = urlunsplit(
        ("https", _host_header(hostname), parts.path or "/", parts.query, "")
    )
    return normalised, hostname, port


def _default_resolver(hostname: str, port: int) -> tuple[str, ...]:
    records = socket.getaddrinfo(hostname, port, family=socket.AF_UNSPEC, type=socket.SOCK_STREAM)
    return tuple(str(record[4][0]) for record in records)


def _is_global_route(address: ipaddress.IPv4Address | ipaddress.IPv6Address) -> bool:
    if not address.is_global:
        return False
    if not isinstance(address, ipaddress.IPv6Address):
        return True

    # Transition formats can carry a second destination inside an otherwise
    # global IPv6 address. Validate that embedded route as well. Teredo and the
    # local-use NAT64 prefix introduce relay or operator-specific routing that
    # cannot be pinned by this process, so safe fetch rejects them outright.
    if address.ipv4_mapped is not None:
        return address.ipv4_mapped.is_global
    if address.sixtofour is not None:
        return address.sixtofour.is_global
    if address.teredo is not None or address in _NAT64_LOCAL_USE:
        return False
    if address in _NAT64_WELL_KNOWN:
        return ipaddress.IPv4Address(int(address) & 0xFFFFFFFF).is_global
    return True


def _validated_target(url: str, resolver: Resolver) -> _Target:
    normalised, hostname, port = _normalise_url(url)
    try:
        answers = tuple(resolver(hostname, port))
    except (OSError, ValueError) as exc:
        raise FetchError(f"DNS resolution failed for {hostname}: {exc}") from exc
    if not answers:
        raise FetchError(f"DNS returned no addresses for {hostname}")

    validated: list[str] = []
    for raw_address in answers:
        address_text = str(raw_address)
        if "%" in address_text:
            raise FetchError(f"DNS returned scoped address for {hostname}: {address_text}")
        try:
            address = ipaddress.ip_address(address_text)
        except ValueError as exc:
            raise FetchError(f"DNS returned invalid address for {hostname}: {address_text}") from exc
        if not _is_global_route(address):
            raise FetchError(f"DNS returned non-global address for {hostname}: {address_text}")
        canonical = str(address)
        if canonical not in validated:
            validated.append(canonical)

    return _Target(normalised, hostname, tuple(validated), port)


class _PinnedHTTPSAdapter(HTTPAdapter):
    """Connect to one validated IP while retaining the URL host for TLS."""

    def __init__(self, connect_ip: str, tls_hostname: str, port: int = 443):
        self._connect_ip = connect_ip
        self._tls_hostname = tls_hostname
        self._port = port
        super().__init__()

    def get_connection_with_tls_context(self, request, verify, proxies=None, cert=None):
        if proxies:
            raise requests.exceptions.ProxyError("safe fetch does not permit proxy routing")
        _host_params, pool_kwargs = self.build_connection_pool_key_attributes(
            request,
            verify,
            cert,
        )
        pool_kwargs["server_hostname"] = self._tls_hostname
        pool_kwargs["assert_hostname"] = self._tls_hostname
        return self.poolmanager.connection_from_host(
            scheme="https",
            host=self._connect_ip,
            port=self._port,
            pool_kwargs=pool_kwargs,
        )


def _open_pinned(
    target: _Target,
    headers: Mapping[str, str],
) -> tuple[requests.Response, Callable[[], None]]:
    # Try each validated address. A host whose first A record is unreachable
    # still has the others, and every attempt pins the same way, so the
    # validation the resolver did is not weakened by trying more of its answers.
    last: Exception | None = None
    for connect_ip in target.connect_ips:
        try:
            return _open_pinned_to(target, connect_ip, headers)
        except requests.exceptions.ProxyError:
            raise
        except requests.RequestException as exc:
            last = exc
    assert last is not None  # _validated_target never returns an empty set
    raise last


# Pinned sessions kept per thread, so consecutive requests to one host reuse
# a keep-alive connection instead of paying a TCP and TLS handshake each
# (measured on Yupoo: about 1.7 s a request fresh, 0.9 s reused). A scrape
# worker handles one seller at a time, which needs its seller host and the
# cover host; a few more cover a redirect. Evicted sessions are closed.
# The key includes the validated address, so a host that resolves elsewhere
# gets a new session pinned there: reuse never skips validation.
_SESSIONS = threading.local()
_MAX_SESSIONS_PER_THREAD = 4


def _pinned_session(target: _Target, connect_ip: str) -> requests.Session:
    cache: OrderedDict[tuple[str, str, int], requests.Session] | None = getattr(
        _SESSIONS, "cache", None)
    if cache is None:
        cache = _SESSIONS.cache = OrderedDict()
    key = (target.hostname, connect_ip, target.port)
    session = cache.pop(key, None)
    if session is None:
        session = requests.Session()
        # Environment proxy settings would create a second routing decision
        # after validation, so this boundary always makes the validated
        # direct connection.
        session.trust_env = False
        session.mount(
            "https://",
            _PinnedHTTPSAdapter(connect_ip, target.hostname, target.port),
        )
    cache[key] = session  # most recently used last
    while len(cache) > _MAX_SESSIONS_PER_THREAD:
        cache.popitem(last=False)[1].close()
    return session


def _open_pinned_to(
    target: _Target,
    connect_ip: str,
    headers: Mapping[str, str],
) -> tuple[requests.Response, Callable[[], None]]:
    session = _pinned_session(target, connect_ip)
    try:
        response = session.get(
            target.url,
            headers=dict(headers),
            timeout=_TIMEOUT,
            allow_redirects=False,
            stream=True,
            proxies={},
        )
    except Exception:
        # A failed connection may be a stale one; start clean next time.
        _SESSIONS.cache.pop((target.hostname, connect_ip, target.port), None)
        session.close()
        raise

    # Closing a fully read response returns its connection to the session's
    # pool; one abandoned mid-body closes the connection instead.
    return response, response.close


def _content_type(response: requests.Response) -> str:
    return response.headers.get("Content-Type", "").split(";", 1)[0].strip().lower()


def _verify_image(path: Path, mime: str) -> None:
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("error", Image.DecompressionBombWarning)
            with Image.open(path) as image:
                image_format = image.format or ""
                width, height = image.size
                if width <= 0 or height <= 0:
                    raise _InvalidImageError("image has invalid dimensions")
                if width > _MAX_IMAGE_DIMENSION or height > _MAX_IMAGE_DIMENSION:
                    raise _InvalidImageError("image dimensions exceed limit")
                if width * height > _MAX_IMAGE_PIXELS:
                    raise _InvalidImageError("image pixel count exceeds limit")
                if mime not in _IMAGE_MIMES.get(image_format, frozenset()):
                    raise _InvalidImageError("image MIME does not match decoded format")
                image.verify()
            # verify() checks structure without decoding the pixels. Re-open and
            # load within the already-checked pixel bound to reject truncated data.
            with Image.open(path) as image:
                image.load()
    except _InvalidImageError:
        raise
    except (
        Image.DecompressionBombError,
        Image.DecompressionBombWarning,
        UnidentifiedImageError,
        OSError,
        ValueError,
    ) as exc:
        raise _InvalidImageError(f"invalid image: {exc}") from exc


class Client:
    def __init__(self, limiter: RateLimiter, user_agent: str = USER_AGENT,
                 resolver: Resolver | None = None, transport: Transport | None = None,
                 sleep_fn=time.sleep):
        self._limiter = limiter
        self._resolver = resolver or _default_resolver
        self._transport = transport or _open_pinned
        self._headers = {"User-Agent": user_agent}
        self._sleep = sleep_fn

    def _follow_redirects(
        self,
        url: str,
        referer: str | None,
    ) -> tuple[requests.Response, Callable[[], None]]:
        current_url = url
        for hop in range(_MAX_REDIRECTS + 1):
            target = _validated_target(current_url, self._resolver)
            headers = dict(self._headers)
            headers["Host"] = _host_header(target.hostname)
            if referer:
                headers["Referer"] = referer
            self._limiter.wait(target.hostname)
            try:
                response, close = self._transport(target, headers)
            except requests.RequestException as exc:
                raise _RetryableFetchError(f"request failed for {target.url}: {exc}") from exc
            except OSError as exc:
                raise _RetryableFetchError(f"request failed for {target.url}: {exc}") from exc

            status = response.status_code
            if status in _REDIRECT_STATUSES:
                location = response.headers.get("Location", "").strip()
                close()
                if not location:
                    raise FetchError(f"redirect missing Location for {target.url}")
                if hop >= _MAX_REDIRECTS:
                    raise FetchError(f"too many redirects for {url}")
                current_url = urljoin(target.url, location)
                continue
            if status in (429, 403):
                close()
                raise BlockedError(status)
            if 500 <= status < 600:
                close()
                raise _RetryableFetchError(f"HTTP {status} for {target.url}")
            if status != 200:
                close()
                raise FetchError(f"HTTP {status} for {target.url}")
            return response, close
        raise AssertionError("redirect loop exhausted without returning")

    def _fetch(
        self,
        url: str,
        referer: str | None,
        *,
        buffer: bool,
    ) -> tuple[requests.Response, Callable[[], None] | None]:
        last_error: Exception | None = None
        for attempt in range(_RETRIES):
            try:
                response, close = self._follow_redirects(url, referer)
                if not buffer:
                    return response, close
                try:
                    body = bytearray()
                    for chunk in response.iter_content(chunk_size=65536):
                        if len(body) + len(chunk) > _MAX_HTML_BYTES:
                            raise FetchError(f"HTML body exceeds {_MAX_HTML_BYTES}-byte limit")
                        body.extend(chunk)
                    response._content = bytes(body)
                    response._content_consumed = True
                except requests.RequestException as exc:
                    raise _RetryableFetchError(f"body read failed for {url}: {exc}") from exc
                finally:
                    close()
                return response, None
            except _RetryableFetchError as exc:
                last_error = exc
                if attempt + 1 < _RETRIES:
                    self._sleep(2.0 ** attempt)
        raise FetchError(f"gave up on {url}: {last_error}")

    def get(self, url: str, referer: str | None = None) -> requests.Response:
        response, _close = self._fetch(url, referer, buffer=True)
        return response

    def _write_bounded_image(self, response: requests.Response, dest: Path) -> bool:
        mime = _content_type(response)
        if mime not in {item for values in _IMAGE_MIMES.values() for item in values}:
            return False
        declared_raw = response.headers.get("Content-Length")
        if declared_raw is not None:
            try:
                declared = int(declared_raw)
            except ValueError:
                return False
            if declared < 0 or declared > _MAX_IMAGE_BYTES:
                return False

        dest.parent.mkdir(parents=True, exist_ok=True)
        tmp = dest.with_name(dest.name + ".tmp")
        replaced = False
        try:
            observed = 0
            with open(tmp, "wb") as fh:
                for chunk in response.iter_content(chunk_size=65536):
                    if not chunk:
                        continue
                    observed += len(chunk)
                    if observed > _MAX_IMAGE_BYTES:
                        return False
                    fh.write(chunk)
            if observed == 0:
                return False
            try:
                _verify_image(tmp, mime)
            except _InvalidImageError:
                return False
            os.replace(tmp, dest)
            replaced = True
            return True
        finally:
            if not replaced:
                tmp.unlink(missing_ok=True)

    def download(self, url: str, dest: Path, referer: str | None = None) -> bool:
        """Fetch a bounded image into dest atomically.

        False means this attempt failed, not that dest is absent: a thumbnail
        stored by an earlier scrape stays in place after any failed fetch.
        """
        # Retry a broken stream as a new, independently resolved request. MIME,
        # size and decode failures are deterministic and return immediately.
        for attempt in range(_RETRIES):
            try:
                response, close = self._fetch(url, referer, buffer=False)
            except BlockedError:
                raise
            except FetchError:
                return False
            assert close is not None
            stream_error: requests.RequestException | None = None
            try:
                result = self._write_bounded_image(response, dest)
            except requests.RequestException as exc:
                stream_error = exc
                result = False
            finally:
                close()
            if stream_error is None:
                return result
            if attempt + 1 < _RETRIES:
                self._sleep(2.0 ** attempt)
        return False
