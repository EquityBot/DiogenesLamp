"""Extract taobao/tmall/weidian purchase links from album text."""

from __future__ import annotations

import re
from dataclasses import dataclass
from urllib.parse import urlparse

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
_TRAILING_PUNCT = ".,;:!?)]}>，。！？；：（）】》「」『』、“”’‘"


@dataclass(frozen=True)
class Link:
    platform: str
    url: str


def _hostname(host: str) -> str | None:
    """Hostname of a captured host, parsed so platform checks cannot match a
    lookalike such as evil-tmall.com via a raw string suffix."""
    name = urlparse(f"https://{host}/").hostname
    if name is None:
        return None
    return name.casefold()


def _labels_end_with(name: str, *suffix: str) -> bool:
    labels = name.split(".")
    return len(labels) >= len(suffix) and tuple(labels[-len(suffix) :]) == suffix


def _platform(host: str) -> str:
    name = _hostname(host)
    if name is None:
        return "other"
    if _labels_end_with(name, "tmall", "com"):
        return "tmall"
    if _labels_end_with(name, "taobao", "com"):
        return "taobao"
    if _labels_end_with(name, "weidian", "com"):
        return "weidian"
    return "other"


def _strip_fragment(rest: str) -> str:
    """Drop any #fragment. Album text runs links together with prose, so the
    matcher routinely swallows one; left in place it becomes part of the item
    id ("id=695#detail"), and the same item linked twice stops matching itself.
    """
    return rest.partition("#")[0]


# The URL regex stops at ASCII whitespace but not at control or invisible
# characters (NUL, zero-width spaces, BOMs), which would otherwise flow
# straight into the stored URL: a NUL in a link breaks nothing in JSON but
# poisons dedup keys and any downstream consumer that validates URLs.
# (Characters written as escapes: a literal zero-width char in a source
# file is invisible to the next reader and to most linters.)
_INVISIBLE = re.compile(
    "[\\x00-\\x1f\\x7f\ufeff\u200b\u200c\u200d\u2060]+"
)


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
        rest = _INVISIBLE.sub("", match.group("rest") or "")
        # Chinese prose in album descriptions is frequently pasted adjacent to
        # URLs without spaces. Discard non-ASCII / CJK text and trailing punctuation
        # attached to the URL.
        rest = re.split(r"[^\x21-\x7e]", rest, maxsplit=1)[0].rstrip(_TRAILING_PUNCT)
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
