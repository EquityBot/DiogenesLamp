"""Parsers for Yupoo album list pages and album pages."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from decimal import ROUND_HALF_UP, Decimal, InvalidOperation

from bs4 import BeautifulSoup

_ALBUM_HREF_RE = re.compile(r"/albums/(\d+)")
_INT_RE = re.compile(r"\d+")
_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}")
# Whole yuan with optional thousands separators and a decimal fraction.
# Trailing word-boundary is not required: catalogue titles glue the amount
# to CJK text (¥180黑色, 180元新款).
_AMOUNT = r"(?:[0-9]{1,3}(?:,[0-9]{3})+|[0-9]+)(?:\.[0-9]+)?"
_PRICE_PATTERNS = (
    re.compile(rf"(?:¥|￥)\s*({_AMOUNT})"),
    re.compile(rf"(?i)\b(?:rmb|yuan)\s*({_AMOUNT})"),
    re.compile(rf"({_AMOUNT})\s*(?:元|块)"),
    re.compile(rf"(?i)({_AMOUNT})\s*(?:rmb|yuan)\b"),
    re.compile(r"\b[pP]([0-9]{2,4})(?![0-9])"),
    re.compile(r"\b([0-9]{2,4})[pP]\b"),
)


def _cny_amount(raw: str) -> int | None:
    try:
        value = Decimal(raw.replace(",", "")).quantize(Decimal(1), rounding=ROUND_HALF_UP)
    except InvalidOperation:
        return None
    amount = int(value)
    if 10 <= amount <= 99999:
        return amount
    return None


def parse_price(text: str) -> int:
    """Extract price in CNY/RMB from title or description text (e.g. ¥180, 180元, P180, 180p).

    Returns 0 if no price found or if the rounded whole-yuan value is outside
    sensible catalogue ranges (10..99999).
    """
    for pat in _PRICE_PATTERNS:
        for match in pat.finditer(text):
            amount = _cny_amount(match.group(1))
            if amount is not None:
                return amount
    return 0


@dataclass(frozen=True)
class AlbumStub:
    id: str
    title: str
    thumb_url: str
    photo_count: int


@dataclass(frozen=True)
class AlbumDetail:
    title: str
    description: str
    category: str
    photo_count: int
    date_published: str = ""
    date_modified: str = ""
    price: int = 0


def _soup(html: str) -> BeautifulSoup:
    return BeautifulSoup(html, "html.parser")


def _absolute(url: str) -> str:
    if url.startswith("//"):
        return "https:" + url
    return url
