import struct
import zlib
from io import BytesIO
from pathlib import Path

import pytest
import requests
from PIL import Image

from lamp import http as lamp_http
from lamp.http import (
    _MAX_IMAGE_BYTES,
    _MAX_IMAGE_DIMENSION,
    _MAX_IMAGE_PIXELS,
    _MAX_REDIRECTS,
    Client,
    FetchError,
    RateLimiter,
    _normalise_url,
    _open_pinned,
    _PinnedHTTPSAdapter,
    _Target,
)


class FakeClock:
    def __init__(self):
        self.now = 0.0

    def time(self) -> float:
        return self.now

    def sleep(self, seconds: float) -> None:
        self.now += seconds


class FakeResponse:
    def __init__(
        self,
        status_code: int,
        content: bytes = b"ok",
        *,
        headers: dict[str, str] | None = None,
        chunks: list[bytes | Exception] | None = None,
    ):
        self.status_code = status_code
        self.content = content
        self.headers = headers or {}
        self.text = content.decode(errors="replace")
        self._chunks = chunks if chunks is not None else [content]
        self.closed = False

    def iter_content(self, chunk_size: int):
        del chunk_size
        for chunk in self._chunks:
            if isinstance(chunk, Exception):
                raise chunk
            yield chunk

    def close(self) -> None:
        self.closed = True


class FakeTransport:
    def __init__(self, responses: list[FakeResponse | Exception]):
        self.responses = list(responses)
        self.calls: list[dict] = []

    def __call__(self, target, headers):
        self.calls.append({"target": target, "headers": dict(headers)})
        result = self.responses.pop(0)
        if isinstance(result, Exception):
            raise result
        return result, result.close


def _client(resolver, responses):
    clock = FakeClock()
    transport = FakeTransport(responses)
    client = Client(
        RateLimiter(time_fn=clock.time, sleep_fn=clock.sleep),
        resolver=resolver,
        transport=transport,
        sleep_fn=clock.sleep,
    )
    return client, transport


def _static_resolver(*addresses: str):
    return lambda _host, _port: addresses


def _image_bytes(format_name: str = "PNG", size: tuple[int, int] = (2, 2)) -> bytes:
    buffer = BytesIO()
    Image.new("RGB", size, "red").save(buffer, format=format_name)
    return buffer.getvalue()


def _png_with_dimensions(width: int, height: int) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + kind
            + data
            + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
        )

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    # A deliberately tiny body is enough to reach Pillow's dimension checks;
    # it is not allowed to reach the destination even if a decoder is lenient.
    body = zlib.compress(b"\x00\x00\x00\x00")
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", body) + chunk(b"IEND", b"")


@pytest.mark.parametrize(
    "answers",
    [
        (),
        ("127.0.0.1",),
        ("10.0.0.8",),
        ("169.254.10.20",),
        ("::1",),
        ("fe80::1",),
        ("8.8.8.8", "127.0.0.1"),
        ("2001:4860:4860::8888", "fc00::1"),
        ("2002:7f00:0001::",),
        ("64:ff9b::7f00:1",),
        ("64:ff9b:1::808:808",),
        ("2001:0000:4136:e378:8000:63bf:3fff:fdd2",),
    ],
)
def test_get_rejects_empty_or_non_global_dns_answers(answers):
    client, transport = _client(_static_resolver(*answers), [FakeResponse(200)])

    with pytest.raises(FetchError):
        client.get("https://catalogue.example/albums")

    assert transport.calls == []


@pytest.mark.parametrize(
    ("url", "answer"),
    [
        ("http://catalogue.example/", "8.8.8.8"),
        ("https://user@catalogue.example/", "8.8.8.8"),
        ("https://catalogue.example:8443/", "8.8.8.8"),
        ("https://localhost/", "127.0.0.1"),
        ("https://127.0.0.1/", "127.0.0.1"),
        ("https://[::ffff:127.0.0.1]/", "::ffff:127.0.0.1"),
        ("https://[fe80::1%25ethernet]/", "fe80::1"),
    ],
)
def test_get_rejects_unsafe_url_forms(url, answer):
    client, transport = _client(_static_resolver(answer), [FakeResponse(200)])

    with pytest.raises(FetchError):
        client.get(url)

    assert transport.calls == []


def test_get_pins_validated_ip_and_preserves_original_host():
    client, transport = _client(_static_resolver("8.8.8.8"), [FakeResponse(200)])

    assert client.get("https://catalogue.example/albums").status_code == 200

    call = transport.calls[0]
    assert call["target"].connect_ips == ("8.8.8.8",)
    assert call["target"].hostname == "catalogue.example"
    assert call["headers"]["Host"] == "catalogue.example"


def test_pinned_adapter_connects_to_ip_but_validates_original_tls_hostname():
    class PoolManagerSpy:
        def __init__(self):
            self.kwargs = None

        def connection_from_host(self, **kwargs):
            self.kwargs = kwargs
            return object()

    adapter = _PinnedHTTPSAdapter("8.8.8.8", "catalogue.example")
    spy = PoolManagerSpy()
    adapter.poolmanager = spy
    request = requests.Request("GET", "https://catalogue.example/albums").prepare()

    adapter.get_connection_with_tls_context(request, verify=True, proxies={})

    assert spy.kwargs["host"] == "8.8.8.8"
    assert spy.kwargs["scheme"] == "https"
    assert spy.kwargs["pool_kwargs"]["server_hostname"] == "catalogue.example"
    assert spy.kwargs["pool_kwargs"]["assert_hostname"] == "catalogue.example"


def test_pinned_adapter_refuses_proxy_routing():
    adapter = _PinnedHTTPSAdapter("8.8.8.8", "catalogue.example")
    request = requests.Request("GET", "https://catalogue.example/albums").prepare()

    with pytest.raises(requests.exceptions.ProxyError):
        adapter.get_connection_with_tls_context(
            request,
            verify=True,
            proxies={"https": "http://proxy.example"},
        )


def test_pinned_transport_ignores_environment_proxies_and_disables_auto_redirects(monkeypatch):
    sessions = []

    class SessionSpy:
        def __init__(self):
            self.trust_env = True
            self.calls = []
            self.closed = False
            sessions.append(self)

        def mount(self, prefix, adapter):
            self.mounted = (prefix, adapter)

        def get(self, url, **kwargs):
            self.calls.append((url, kwargs))
            return FakeResponse(200)

        def close(self):
            self.closed = True

    monkeypatch.setattr("lamp.http.requests.Session", SessionSpy)
    response, close = _open_pinned(
        _Target("https://catalogue.example/", "catalogue.example", "8.8.8.8", 443),
        {"Host": "catalogue.example"},
    )

    session = sessions[0]
    assert session.trust_env is False
    assert session.calls[0][1]["proxies"] == {}
    assert session.calls[0][1]["allow_redirects"] is False
    assert session.calls[0][1]["stream"] is True
    close()
    assert response.closed is True
    assert session.closed is True


def test_redirect_is_resolved_again_and_pinned_independently():
    answers = {
        "catalogue.example": ("8.8.8.8",),
        "images.example": ("1.1.1.1",),
    }
    client, transport = _client(
        lambda host, _port: answers[host],
        [
            FakeResponse(302, headers={"Location": "https://images.example/thumb.png"}),
            FakeResponse(200),
        ],
    )

    client.get("https://catalogue.example/start")

    assert [call["target"].connect_ips for call in transport.calls] == [("8.8.8.8",), ("1.1.1.1",)]
    assert [call["headers"]["Host"] for call in transport.calls] == [
        "catalogue.example",
        "images.example",
    ]


def test_redirect_rebinding_to_private_address_is_blocked_before_second_connection():
    answers = iter([("8.8.8.8",), ("127.0.0.1",)])
    client, transport = _client(
        lambda _host, _port: next(answers),
        [FakeResponse(302, headers={"Location": "/again"}), FakeResponse(200)],
    )

    with pytest.raises(FetchError):
        client.get("https://catalogue.example/start")

    assert len(transport.calls) == 1


def test_redirect_https_downgrade_is_blocked():
    client, transport = _client(
        _static_resolver("8.8.8.8"),
        [FakeResponse(302, headers={"Location": "http://catalogue.example/plain"})],
    )

    with pytest.raises(FetchError):
        client.get("https://catalogue.example/start")

    assert len(transport.calls) == 1


def test_redirect_without_location_is_rejected_and_closed():
    redirect = FakeResponse(302)
    client, _ = _client(_static_resolver("8.8.8.8"), [redirect])

    with pytest.raises(FetchError):
        client.get("https://catalogue.example/start")

    assert redirect.closed is True


def test_redirect_hop_limit_is_enforced():
    client, transport = _client(
        _static_resolver("8.8.8.8"),
        [FakeResponse(302, headers={"Location": "/again"}) for _ in range(_MAX_REDIRECTS + 1)],
    )

    with pytest.raises(FetchError):
        client.get("https://catalogue.example/start")

    assert len(transport.calls) == _MAX_REDIRECTS + 1


def test_download_accepts_a_bounded_decodable_image(tmp_path: Path):
    payload = _image_bytes()
    response = FakeResponse(
        200,
        payload,
        headers={"Content-Type": "image/png", "Content-Length": str(len(payload))},
    )
    client, _ = _client(_static_resolver("8.8.8.8"), [response])
    dest = tmp_path / "thumb.png"

    assert client.download("https://images.example/thumb.png", dest) is True
    assert dest.read_bytes() == payload
    assert list(tmp_path.iterdir()) == [dest]


def test_download_accepts_content_type_parameters(tmp_path: Path):
    payload = _image_bytes()
    response = FakeResponse(
        200,
        payload,
        headers={"Content-Type": "Image/PNG; charset=binary"},
    )
    client, _ = _client(_static_resolver("8.8.8.8"), [response])

    assert client.download("https://images.example/thumb.png", tmp_path / "thumb.png") is True


def test_download_rejects_oversized_declared_body_and_preserves_old_image(tmp_path: Path):
    dest = tmp_path / "thumb.png"
    dest.write_bytes(b"old-image")
    response = FakeResponse(
        200,
        _image_bytes(),
        headers={"Content-Type": "image/png", "Content-Length": str(_MAX_IMAGE_BYTES + 1)},
    )
    client, _ = _client(_static_resolver("8.8.8.8"), [response])

    assert client.download("https://images.example/thumb.png", dest) is False
    assert dest.read_bytes() == b"old-image"
    assert sorted(path.name for path in tmp_path.iterdir()) == ["thumb.png"]


def test_download_stops_when_stream_exceeds_observed_byte_cap(tmp_path: Path):
    half = _MAX_IMAGE_BYTES // 2 + 1
    response = FakeResponse(
        200,
        headers={"Content-Type": "image/png"},
        chunks=[b"x" * half, b"y" * half],
    )
    client, _ = _client(_static_resolver("8.8.8.8"), [response])
    dest = tmp_path / "thumb.png"

    assert client.download("https://images.example/thumb.png", dest) is False
    assert not dest.exists()
    assert list(tmp_path.iterdir()) == []


def test_download_rejects_mime_deception(tmp_path: Path):
    payload = _image_bytes("PNG")
    response = FakeResponse(200, payload, headers={"Content-Type": "image/jpeg"})
    client, _ = _client(_static_resolver("8.8.8.8"), [response])

    assert client.download("https://images.example/thumb.jpg", tmp_path / "thumb.jpg") is False
    assert list(tmp_path.iterdir()) == []


def test_download_rejects_excessive_dimension(tmp_path: Path):
    payload = _png_with_dimensions(_MAX_IMAGE_DIMENSION + 1, 1)
    response = FakeResponse(200, payload, headers={"Content-Type": "image/png"})
    client, _ = _client(_static_resolver("8.8.8.8"), [response])

    assert client.download("https://images.example/thumb.png", tmp_path / "thumb.png") is False
    assert list(tmp_path.iterdir()) == []


def test_download_rejects_excessive_pixel_count(tmp_path: Path):
    side = int(_MAX_IMAGE_PIXELS ** 0.5) + 1
    payload = _png_with_dimensions(side, side)
    response = FakeResponse(200, payload, headers={"Content-Type": "image/png"})
    client, _ = _client(_static_resolver("8.8.8.8"), [response])

    assert client.download("https://images.example/thumb.png", tmp_path / "thumb.png") is False
    assert list(tmp_path.iterdir()) == []


def test_download_rejects_truncated_image(tmp_path: Path):
    payload = _image_bytes()[:-12]
    response = FakeResponse(200, payload, headers={"Content-Type": "image/png"})
    client, _ = _client(_static_resolver("8.8.8.8"), [response])

    assert client.download("https://images.example/thumb.png", tmp_path / "thumb.png") is False
    assert list(tmp_path.iterdir()) == []


@pytest.mark.parametrize("url", [
    "https://[::1",
    "https://[abc",
    "https://[::1]:80]/x",
])
def test_a_malformed_authority_raises_fetch_error_not_value_error(url):
    # urlsplit raises ValueError on these. download() contracts on FetchError, so
    # a ValueError escaping the boundary aborted the scrape instead of the URL.
    with pytest.raises(FetchError):
        _normalise_url(url)


def test_every_validated_address_is_tried_before_the_host_fails(monkeypatch):
    # Only the first validated address was ever used, so one unreachable A record
    # failed a host that had a reachable one.
    tried: list[str] = []

    def fake_open(target, connect_ip, headers):
        tried.append(connect_ip)
        if connect_ip != "1.1.1.1":
            raise requests.exceptions.ConnectionError("refused")
        return FakeResponse(200), lambda: None

    monkeypatch.setattr(lamp_http, "_open_pinned_to", fake_open)
    target = _Target("https://example/", "example", ("8.8.8.8", "1.1.1.1"), 443)
    response, _close = _open_pinned(target, {})
    assert response.status_code == 200
    assert tried == ["8.8.8.8", "1.1.1.1"]


def test_the_last_connection_error_is_raised_when_every_address_fails(monkeypatch):
    def fake_open(target, connect_ip, headers):
        raise requests.exceptions.ConnectionError("refused " + connect_ip)

    monkeypatch.setattr(lamp_http, "_open_pinned_to", fake_open)
    target = _Target("https://example/", "example", ("8.8.8.8", "1.1.1.1"), 443)
    with pytest.raises(requests.exceptions.ConnectionError, match="1.1.1.1"):
        _open_pinned(target, {})


def test_a_proxy_error_stops_immediately_rather_than_trying_more_addresses(monkeypatch):
    # Proxy routing is refused by design, so it must not look like one bad address.
    tried: list[str] = []

    def fake_open(target, connect_ip, headers):
        tried.append(connect_ip)
        raise requests.exceptions.ProxyError("safe fetch does not permit proxy routing")

    monkeypatch.setattr(lamp_http, "_open_pinned_to", fake_open)
    target = _Target("https://example/", "example", ("8.8.8.8", "1.1.1.1"), 443)
    with pytest.raises(requests.exceptions.ProxyError):
        _open_pinned(target, {})
    assert tried == ["8.8.8.8"]
