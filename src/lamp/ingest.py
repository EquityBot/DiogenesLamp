"""Ingest raw link dumps into sources.toml."""

from __future__ import annotations

import json
import os
import re
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import parse_qs, urlparse

_YUPOO_SUFFIX = ".x.yupoo.com"


@dataclass(frozen=True)
class Source:
    kind: str
    id: str
    platform: str
    url: str
    name: str
    # Set skip = true in sources.toml to leave a seller out of scrapes -
    # useful for a catalogue too large to be worth the hours, or one that no
    # longer sells anything you want. Commenting the entry out would not
    # last: import rewrites the file and would restore it from the dump.
    skip: bool = False


def _slug(text: str) -> str:
    slug = re.sub(r"[^a-z0-9-]+", "-", text.lower()).strip("-")
    return re.sub(r"-{2,}", "-", slug)


def classify_url(raw: str) -> Source | None:
    raw = raw.strip()
    if (raw.startswith('"') and raw.endswith('"')) or (raw.startswith("'") and raw.endswith("'")):
        raw = raw[1:-1].strip()
    if not raw or raw.upper() == "N/A":
        return None
    # The dump is hand-exported text: a NUL or control char would flow into
    # sources.toml and from there into request URLs and the shop directory.
    # Same invisible set links.py scrubs, written as escapes so the source
    # file shows what it strips.
    raw = re.sub("[\\x00-\\x1f\\x7f\ufeff\u200b\u200c\u200d\u2060]+", "", raw)
    if not raw:
        return None
    if "://" not in raw:
        raw = "https://" + raw
    # A hand-exported dump carries malformed authorities: an unclosed bracket,
    # a bracketed host that is not an IP address, a fullwidth slash. urlparse
    # and its hostname accessor raise ValueError on those. One bad row is
    # invalid input, not a reason to stop the whole import.
    try:
        parsed = urlparse(raw)
        if parsed.scheme.lower() not in {"http", "https"}:
            return None
        if parsed.username is not None or parsed.password is not None:
            return None
        host = (parsed.hostname or "").lower()
    except ValueError:
        return None
    if not host or "." not in host:
        return None

    if host.endswith(_YUPOO_SUFFIX):
        seller = host[: -len(_YUPOO_SUFFIX)]
        if not seller:
            return None
        # Dots in a multi-label subdomain would become nested TOML tables and
        # crash load_sources, so the id is always a single slug label.
        seller_id = _slug(seller.replace(".", "-"))
        return Source("yupoo", seller_id, "yupoo", f"https://{host}", seller_id)

    if host == "taobao.com" or host.endswith(".taobao.com"):
        stem = host.removesuffix(".taobao.com") or "taobao"
        return Source("shop", f"taobao-{_slug(stem)}", "taobao", f"https://{host}/", stem)

    if host == "weidian.com":
        userid = parse_qs(parsed.query).get("userid", [None])[0]
        # A weidian userid is numeric. A non-numeric value (or none at all) would
        # produce a TOML-unsafe id, so it falls through to the generic web fallback.
        if userid and re.fullmatch(r"[0-9]+", userid):
            return Source(
                "shop", f"weidian-{userid}", "weidian",
                f"https://weidian.com/?userid={userid}", f"weidian {userid}",
            )
        return Source("shop", "weidian-com", "web", raw, "weidian.com")

    if host == "k.youshop10.com":
        code = parsed.path.strip("/")
        if not code:
            return None
        return Source("shop", f"youshop-{_slug(code)}", "weidian-shortlink", raw, code)

    if host in ("vlink.cc", "linktr.ee"):
        stem = host.split(".")[0]
        page = _slug(parsed.path.strip("/")) or "home"
        return Source("shop", f"{stem}-{page}", "aggregator", raw, f"{stem}/{parsed.path.strip('/')}")

    name = host.removeprefix("www.")
    return Source("shop", _slug(name), "web", raw, name)


@dataclass
class ImportReport:
    rows: int = 0
    invalid: int = 0
    yupoo_new: int = 0
    yupoo_known: int = 0
    shops_new: int = 0
    shops_known: int = 0
    shops_by_platform: dict[str, int] = field(default_factory=dict)
    # Distinct hosts can slug to the same id: a yupoo seller "foo.bar" and a
    # seller "foo-bar" both become "foo-bar", because a dot cannot stay in a TOML
    # key. Counting the second as already known would drop a real seller with no
    # trace, so each collision is named here instead.
    collisions: list[tuple[str, str, str]] = field(default_factory=list)


def parse_dump(text: str) -> list[str]:
    rows: list[str] = []
    for line in text.splitlines():
        line = line.strip().lstrip("\ufeff")
        if not line:
            continue
        # Tolerate "123<TAB>url" and "123,url" index prefixes, including
        # quoted variants ('"123",url', '123,"url"', '"123","url"').
        # Splitting on every comma and keeping the last field would silently
        # mangle any URL that itself contains a comma (weidian search links do),
        # so only a run of purely numeric fields is stripped.
        match = re.match(r'^(?:(?:"[0-9]+"|[0-9]+)[\t,]\s*)+(?P<url>.+)$', line)
        extracted = (match.group("url") if match else line).strip()
        if (extracted.startswith('"') and extracted.endswith('"')) or (
            extracted.startswith("'") and extracted.endswith("'")
        ):
            extracted = extracted[1:-1].strip()
        rows.append(extracted)
    return rows


def merge_sources(
    existing: dict[str, Source], raws: list[str]
) -> tuple[dict[str, Source], ImportReport]:
    merged = dict(existing)
    report = ImportReport(rows=len(raws))
    seen_this_run: set[str] = set()
    for raw in raws:
        src = classify_url(raw)
        if src is None:
            report.invalid += 1
            continue
        key = f"{src.kind}:{src.id}"
        if key in merged or key in seen_this_run:
            held = merged.get(key)
            if held is not None and held.url != src.url:
                report.collisions.append((key, held.url, src.url))
            if key not in seen_this_run:
                if src.kind == "yupoo":
                    report.yupoo_known += 1
                else:
                    report.shops_known += 1
            seen_this_run.add(key)
            continue
        merged[key] = src
        seen_this_run.add(key)
        if src.kind == "yupoo":
            report.yupoo_new += 1
        else:
            report.shops_new += 1
            report.shops_by_platform[src.platform] = (
                report.shops_by_platform.get(src.platform, 0) + 1
            )
    return merged, report


def _toml_str(value: str) -> str:
    return json.dumps(value, ensure_ascii=False).replace("\x7f", "\\u007f")


_SAFE_ID = re.compile(r"[A-Za-z0-9_-]+")


def write_sources(path: Path, sources: dict[str, Source]) -> None:
    lines = [
        "# Managed by 'lamp import'. Manual edits to name/url are preserved.",
        "# Add 'skip = true' to an entry to leave it out of scrapes.",
        "",
    ]
    for key in sorted(sources):
        src = sources[key]
        if not _SAFE_ID.fullmatch(src.id):
            # A bare TOML key only allows [A-Za-z0-9_-]. Anything else would produce
            # a nested table or invalid TOML and silently corrupt the file, so fail
            # loudly here instead of writing a file load_sources can't parse back.
            raise ValueError(f"unsafe TOML id: {src.id!r}")
        section = "yupoo" if src.kind == "yupoo" else "shops"
        lines.append(f"[{section}.{src.id}]")
        if src.kind != "yupoo":
            lines.append(f"platform = {_toml_str(src.platform)}")
        lines.append(f"url = {_toml_str(src.url)}")
        lines.append(f"name = {_toml_str(src.name)}")
        if src.skip:
            lines.append("skip = true")
        lines.append("")
    tmp = path.with_suffix(".toml.tmp")
    tmp.write_text("\n".join(lines), encoding="utf-8")
    os.replace(tmp, path)


def load_sources(path: Path) -> dict[str, Source]:
    if not path.exists():
        return {}
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    sources: dict[str, Source] = {}
    for seller_id, entry in data.get("yupoo", {}).items():
        if not _SAFE_ID.fullmatch(seller_id):
            raise ValueError(f"unsafe TOML id: {seller_id!r}")
        # Trailing slash on a hand-edited url makes every request URL carry
        # "//albums" - Yupoo answers with a 302 to the canonical path, so
        # the whole run pays a redirect round-trip per page for a typo.
        url = entry["url"].rstrip("/")
        sources[f"yupoo:{seller_id}"] = Source(
            "yupoo", seller_id, "yupoo", url, entry.get("name", seller_id),
            skip=bool(entry.get("skip", False)),
        )
    for shop_id, entry in data.get("shops", {}).items():
        if not _SAFE_ID.fullmatch(shop_id):
            raise ValueError(f"unsafe TOML id: {shop_id!r}")
        sources[f"shop:{shop_id}"] = Source(
            "shop", shop_id, entry.get("platform", "web"), entry["url"],
            entry.get("name", shop_id), skip=bool(entry.get("skip", False)),
        )
    return sources
