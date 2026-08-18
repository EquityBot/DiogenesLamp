"""Extract taobao/tmall/weidian purchase links from album text."""

from __future__ import annotations

import re
from dataclasses import dataclass

# scheme-optional URL matcher for the hosts we care about; the lookarounds stop
# matches starting or ending inside a longer word ("notataobao.com", "taobao.company")
_URL_RE = re.compile(
    r"(?<![\w-])"
    r"(?:https?://)?"
    r"(?P<host>(?:[\w-]+\.)*(?:taobao\.com|tmall\.com|weidian\.com|m\.tb\.cn|k\.youshop10\.com))"
    r"(?![\w-])"
    r"(?P<rest>/[^\s'\"<>]*|\?[^\s'\"<>]*)?",
    re.IGNORECASE,
)

_KEEP_PARAMS = {"id", "itemid", "userid"}
_SHORTLINK_HOSTS = ("m.tb.cn", "k.youshop10.com")
# Album text routinely wraps or ends links with prose punctuation - strip it
# from the tail of the matched rest before it reaches the URL, not the regex.
_TRAILING_PUNCT = ".,;:!?)"


@dataclass(frozen=True)
class Link:
    platform: str
    url: str


def _platform(host: str) -> str:
    if host.endswith("tmall.com"):
        return "tmall"
    if host.endswith("taobao.com"):
        return "taobao"
    if host.endswith("weidian.com"):
        return "weidian"
    return "other"


def _strip_fragment(rest: str) -> str:
    """Drop any #fragment. Album text runs links together with prose, so the
    matcher routinely swallows one; left in place it becomes part of the item
    id ("id=695#detail"), and the same item linked twice stops matching itself.
    """
    return rest.partition("#")[0]


def _normalize(host: str, rest: str) -> str:
    path, _, query = _strip_fragment(rest or "/").partition("?")
    kept = [
        pair for pair in query.split("&")
        if pair and pair.split("=", 1)[0].lower() in _KEEP_PARAMS
    ]
    url = f"https://{host}{path or '/'}"
    if kept:
        url += "?" + "&".join(kept)
    return url


def extract_links(text: str) -> list[Link]:
    seen: set[str] = set()
    links: list[Link] = []
    for match in _URL_RE.finditer(text):
        host = match.group("host").lower()
        rest = (match.group("rest") or "").rstrip(_TRAILING_PUNCT)
        platform = _platform(host)
        if platform == "other":
            if host not in _SHORTLINK_HOSTS:
                continue
            url = f"https://{host}{_strip_fragment(rest)}"
        else:
            url = _normalize(host, rest)
        if url in seen:
            continue
        seen.add(url)
        links.append(Link(platform, url))
    return links
