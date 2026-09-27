"""Extract purchase links from album text.

Marketplace links (taobao, tmall, weidian, 1688) are normalised to the item
they name. Shopping-agent links (kakobuy, hoobuy and the rest) wrap the same
marketplace item, so they are unwrapped to it: the record carries the
marketplace link, which outlives the agent (several closed in 2024 and 2026),
and the viewer rebuilds a link for whichever agent the operator prefers.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from urllib.parse import parse_qsl, urlparse

_MARKETPLACE_HOSTS = ("taobao.com", "tmall.com", "weidian.com", "1688.com")
_SHORTLINK_HOSTS = ("m.tb.cn", "k.youshop10.com")
# Agents whose product links carry a marketplace item. The viewer's agents.js
# lists the same hosts (a parity test keeps the two equal) and holds the URL
# templates that rebuild links for the agents still trading. The formats
# below were read from the public link converters on 27 September 2026.
_AGENT_HOSTS = (
    "kakobuy.com", "hoobuy.com", "oopbuy.com", "mulebuy.com", "joyagoo.com",
    "cssbuy.com", "sugargoo.com", "superbuy.com", "loongbuy.com", "pikobuy.com",
    "cnfans.com", "allchinabuy.com", "pandabuy.com",
    "itaobuy.com", "bbdbuy.com", "ootdbuy.com", "lovegobuy.com", "cnshopper.com",
    "alleurobuy.com", "rizzitgo.com", "vigorbuy.com", "usfans.com", "litbuy.com",
    "hipobuy.com", "gtbuy.com",
)

_HOST_ALTERNATION = "|".join(
    re.escape(host) for host in (*_MARKETPLACE_HOSTS, *_SHORTLINK_HOSTS, *_AGENT_HOSTS)
)
# scheme-optional URL matcher for the hosts we care about. Lookarounds and host
# labels are ASCII-only: Python \w includes CJK, so a Unicode word boundary
# either misses a scheme-less URL glued to Chinese prose ("购买weidian.com/...")
# or swallows that prose into the hostname ("链接item.taobao.com" -> a
# malformed https://链接item.taobao.com/...). ASCII labels still stop matches
# inside a longer English word ("notataobao.com", "taobao.company").
_URL_RE = re.compile(
    # An "@" on either side means the host sits in or after a userinfo part
    # (weidian.com@evil.example, user@weidian.com): the candidate is refused
    # whole rather than repaired by dropping the credentials.
    r"(?<![A-Za-z0-9@-])"
    r"(?:https?://)?"
    r"(?P<host>(?:[A-Za-z0-9-]+\.)*(?:" + _HOST_ALTERNATION + r"))"
    # A following dot must not end the host either: taobao.com.evil.example would
    # otherwise match host="taobao.com" with no rest, and the lookalike domain
    # would be stored as a link to the real homepage. A trailing dot before prose
    # still matches, because no label follows it.
    r"(?!\.?[A-Za-z0-9-]|@)"
    r"(?P<rest>/[^\s'\"<>]*|\?[^\s'\"<>]*)?",
    re.IGNORECASE,
)

_KEEP_PARAMS = {"id", "itemid", "userid"}
# Album text routinely wraps or ends links with prose punctuation - strip it
# from the tail of the matched rest before it reaches the URL, not the regex.
_TRAILING_PUNCT = ".,;:!?)]}>，。！？；：（）】》「」『』、“”’‘"

# The item page each marketplace serves for a bare item id.
ITEM_URLS = {
    "taobao": "https://item.taobao.com/item.htm?id={id}",
    "tmall": "https://detail.tmall.com/item.htm?id={id}",
    "weidian": "https://weidian.com/item.html?itemID={id}",
    "1688": "https://detail.1688.com/offer/{id}.html",
}
# How agents name a marketplace in a channel, shop_type or source parameter.
_AGENT_PLATFORMS = {
    "weidian": "weidian", "wd": "weidian", "taobao": "taobao", "tb": "taobao",
    "tmall": "tmall", "1688": "1688", "ali_1688": "1688", "alibaba": "1688",
    "tao": "taobao", "ali": "1688",
}
# Some agents name the marketplace in the path: /product/<channel>/<id>. hoobuy
# and vigorbuy number the channels one way, usfans another, and litbuy, hipobuy
# and gtbuy spell weidian out; hipobuy ends a 1688 path with .html.
_HOOBUY_CHANNELS = {"0": "1688", "1": "taobao", "2": "weidian"}
_USFANS_CHANNELS = {"1": "1688", "2": "taobao", "3": "weidian"}
_NAMED_CHANNELS = {"0": "1688", "1": "taobao", "weidian": "weidian"}
_PATH_CHANNELS = {
    "hoobuy.com": _HOOBUY_CHANNELS, "vigorbuy.com": _HOOBUY_CHANNELS,
    "usfans.com": _USFANS_CHANNELS, "litbuy.com": _NAMED_CHANNELS,
    "hipobuy.com": _NAMED_CHANNELS, "gtbuy.com": _NAMED_CHANNELS,
}
# rizzitgo numbers the marketplace in its source parameter.
_RIZZITGO_SOURCES = {"1": "taobao", "2": "1688", "3": "weidian"}
# cssbuy names the marketplace in the file name: item-<id>, item-micro-<id>, item-1688-<id>.
_CSSBUY_PREFIXES = {"": "taobao", "micro-": "weidian", "1688-": "1688"}
# Parameters that carry the whole marketplace URL, percent-encoded.
_WRAPPED_URL_PARAMS = ("url", "productlink", "producturl", "link")
_ITEM_ID_RE = re.compile(r"[0-9]{1,24}")


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
    for platform in ITEM_URLS:
        if _labels_end_with(name, platform, "com"):
            return platform
    return "other"


def _agent(host: str) -> str:
    """The agent domain a host belongs to, or '' for any other host."""
    name = _hostname(host)
    if name is None:
        return ""
    for agent in _AGENT_HOSTS:
        if _labels_end_with(name, *agent.split(".")):
            return agent
    return ""


def _strip_fragment(rest: str) -> str:
    """Drop any #fragment. Album text runs links together with prose, so the
    matcher routinely swallows one; left in place it becomes part of the item
    id ("id=695#detail"), and the same item linked twice stops matching itself.
    """
    return rest.partition("#")[0]


# The URL regex stops at ASCII whitespace but not at control or invisible
# characters (NUL, zero-width spaces, BOMs), which would otherwise flow
# straight into the stored URL: a NUL in a link breaks nothing in JSON but
# poisons dedup keys and any downstream consumer that validates URLs. They
# end the URL the way a space does; deleting them instead would splice
# "id=12<NUL>34" into an id nobody wrote.
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


def item_link(platform: str, item_id: str) -> Link | None:
    """The marketplace link for a bare item id, or None for anything malformed."""
    if platform not in ITEM_URLS or not _ITEM_ID_RE.fullmatch(item_id):
        return None
    return Link(platform, ITEM_URLS[platform].format(id=item_id))


def _unwrap_agent(agent: str, rest: str, depth: int) -> Link | None:
    """The marketplace item an agent product link points at.

    Links are rebuilt from the bare item id (or the wrapped marketplace URL),
    never copied, so an agent's own parameters and referral codes fall away.
    A wrapped URL goes back through extract_links, which applies the same
    host rules: an agent link wrapping some other site yields nothing.
    """
    path, _, query = _strip_fragment(rest or "/").partition("?")
    values: dict[str, set[str]] = {}
    for key, value in parse_qsl(query):
        if value:
            values.setdefault(key.lower(), set()).add(value)
    # A parameter repeated with different values names no single item:
    # refuse the ambiguity rather than trust whichever came first.
    if any(len(found) > 1 for found in values.values()):
        return None
    params = {key: next(iter(found)) for key, found in values.items()}
    for key in _WRAPPED_URL_PARAMS:
        wrapped = params.get(key, "")
        if wrapped:
            # parse_qsl decoded the one layer the agent applied. The wrapped
            # value is one whole URL, judged by its own host: a marketplace
            # URL riding inside some other site's query string is that site's
            # business, not a purchase link.
            return _wrapped_link(wrapped, depth + 1)
    ids = {params[key] for key in ("id", "itemid", "goodsid", "keyword") if key in params}
    channels = {params[key].lower() for key in ("channel", "shop_type", "source", "platform", "type")
                if key in params}
    if len(ids) > 1 or len(channels) > 1:
        return None
    if ids and channels:
        names = _RIZZITGO_SOURCES if agent == "rizzitgo.com" else _AGENT_PLATFORMS
        return item_link(names.get(next(iter(channels)), ""), next(iter(ids)))
    codes = _PATH_CHANNELS.get(agent)
    if codes:
        match = re.fullmatch(r"/product/([A-Za-z0-9]+)/([0-9]+)(?:\.html)?/?", path)
        if match:
            return item_link(codes.get(match.group(1).lower(), ""), match.group(2))
    if agent == "cssbuy.com":
        match = re.fullmatch(r"/item-(micro-|1688-)?([0-9]+)\.html", path)
        if match:
            return item_link(_CSSBUY_PREFIXES[match.group(1) or ""], match.group(2))
    return None


def _wrapped_link(url_text: str, depth: int) -> Link | None:
    """The item one whole wrapped URL names: a marketplace item page, or one
    layer deeper an agent link. Anything else, including an unrelated site
    carrying a marketplace URL in its own query string, names nothing.
    Two layers of wrapping is one agent quoting another; deeper nesting only
    comes from crafted text, and stops here.
    """
    match = _URL_RE.fullmatch(url_text.strip())
    if match is None:
        return None
    host = match.group("host").lower()
    rest = _strip_fragment(_clean_rest(match.group("rest")))
    platform = _platform(host)
    if platform != "other":
        return Link(platform, _normalize(host, rest))
    agent = _agent(host)
    if agent and depth < 2:
        return _unwrap_agent(agent, rest, depth)
    return None


def _clean_rest(rest: str | None) -> str:
    """The path and query of a candidate, ended at the first invisible or
    non-ASCII character and shorn of trailing prose punctuation. Chinese
    prose in album descriptions is frequently pasted adjacent to URLs without
    spaces, and the same text arrives percent-encoded inside agent links.
    """
    rest = _INVISIBLE.split(rest or "", maxsplit=1)[0]
    return re.split(r"[^\x21-\x7e]", rest, maxsplit=1)[0].rstrip(_TRAILING_PUNCT)


def extract_links(text: str) -> list[Link]:
    seen: set[str] = set()
    links: list[Link] = []
    for match in _URL_RE.finditer(text):
        host = match.group("host").lower()
        rest = _clean_rest(match.group("rest"))
        platform = _platform(host)
        if platform == "other":
            agent = _agent(host)
            if agent:
                link = _unwrap_agent(agent, rest, 0)
                if link is None or link.url in seen:
                    continue
                seen.add(link.url)
                links.append(link)
                continue
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
