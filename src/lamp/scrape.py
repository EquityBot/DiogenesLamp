"""Scrape Yupoo sellers into catalogue shards."""

from __future__ import annotations

import queue
import re
import sys
import threading
import traceback
from collections.abc import Callable
from contextlib import nullcontext
from dataclasses import asdict, dataclass, field
from datetime import UTC, date, datetime
from pathlib import Path

from lamp.age import is_stale
from lamp.covers import cover_facts
from lamp.http import BlockedError, Client, FetchError, RateLimiter
from lamp.ingest import Source
from lamp.links import extract_links
from lamp.locking import catalogue_lock
from lamp.parse import AlbumStub, parse_album_list, parse_album_page, price_evidence
from lamp.storage import (
    load_shard,
    merge_album,
    read_run,
    record_change,
    write_index,
    write_run,
    write_shard,
)

# Stored album text: enough for product codes, sizes and batch names, small
# enough that 40,000 records add megabytes rather than tens of them.
DESCRIPTION_LIMIT = 500
_BLOCKED_STREAK_LIMIT = 3
_THUMB_BLOCKED_STREAK_LIMIT = 3


_ALBUM_LINK_RE = re.compile(r"/albums/\d+")
# Distinct album links on a card-less page before we call it a parser
# problem rather than an empty seller.
_LAYOUT_CHANGE_MIN_LINKS = 3


class LayoutChangeError(Exception):
    """Page carries albums the parser could not read - selectors need work."""


class EmptySellerError(Exception):
    """Page carries no albums at all - the seller is empty, closed, or gone."""


class _Stopped(Exception):
    """The operator interrupted the run before this seller's listing finished."""


@dataclass
class SellerSummary:
    seller: str
    scraped: int = 0
    skipped: int = 0
    failed: int = 0
    error: str = ""
    failed_ids: list[str] = field(default_factory=list)
    thumbs_blocked: bool = False
    dated: int = 0  # albums whose page yielded a JSON-LD publish date
    skipped_old: int = 0  # albums past the age cutoff, never fetched
    thumbs_refetched: int = 0  # covers of unchanged albums, missing or changed
    refreshed: int = 0
    deferred: int = 0
    listing_complete: bool = field(default=False, compare=False)
    resolved_ids: list[str] = field(default_factory=list, compare=False, repr=False)


class RefreshBudget:
    def __init__(self, limit: int):
        self.remaining = limit
        self.lock = threading.Lock()

    def take(self) -> bool:
        with self.lock:
            if self.remaining <= 0:
                return False
            self.remaining -= 1
            return True


def detail_due(album: dict, run_ts: str, days: int) -> bool:
    if days <= 0:
        return False
    try:
        previous = datetime.fromisoformat(album.get("updated_at", ""))
        now = datetime.fromisoformat(run_ts)
        if previous.tzinfo is None:
            previous = previous.replace(tzinfo=UTC)
        if now.tzinfo is None:
            now = now.replace(tzinfo=UTC)
        return (now - previous).total_seconds() >= days * 86400
    except (ValueError, TypeError):
        return True


def _fetch_all_stubs(client, source: Source, data_dir: Path, save_debug: bool = True,
                     stop: threading.Event | None = None):
    stubs: list[AlbumStub] = []
    seen_ids: set[str] = set()
    page = 1
    while True:
        if stop is not None and stop.is_set():
            raise _Stopped
        url = f"{source.url}/albums?page={page}"
        html = client.get(url, referer=source.url).text
        batch = parse_album_list(html)
        if not batch:
            if page == 1:
                debug = data_dir / "debug" / f"{source.id}-page1.html"
                saved_note = ""
                if save_debug:
                    debug.parent.mkdir(parents=True, exist_ok=True)
                    debug.write_text(html, encoding="utf-8")
                    saved_note = f"; saved {debug}"
                # Distinguish the 2 causes, because they need opposite
                # responses: a page with album links we failed to read is a
                # parser problem worth investigating, while a page with none
                # is simply an empty or closed seller. Blaming a layout
                # change for both once hid every seller on Yupoo's second
                # template behind an error that read like dead accounts.
                # Several links, not one: a page with no albums can still
                # carry a single /albums/ href in a nav or promo block, and
                # calling that a layout change sends the reader hunting for
                # a parser bug that is not there.
                if len(set(_ALBUM_LINK_RE.findall(html))) >= _LAYOUT_CHANGE_MIN_LINKS:
                    raise LayoutChangeError(
                        f"page 1 of {source.id} has album links but parsed to "
                        f"zero albums{saved_note} - Yupoo layout change?"
                    )
                raise EmptySellerError(
                    f"page 1 of {source.id} contains no albums{saved_note} "
                    f"- empty or closed seller?"
                )
            return stubs
        # Defensive hardening for seller variance across the production run: a
        # real Yupoo past-the-end page comes back empty (confirmed live), but a
        # hypothetical seller could instead repeat the last page's content
        # forever. If nothing on this page is new, treat it the same as an
        # empty page - end of catalogue, not an error - rather than looping.
        new_stubs = [stub for stub in batch if stub.id not in seen_ids]
        if not new_stubs:
            return stubs
        stubs.extend(new_stubs)
        seen_ids.update(stub.id for stub in new_stubs)
        page += 1


def scrape_seller(client, source: Source, data_dir: Path,
                  run_ts: str, full: bool = False,
                  max_age_days: int = 0, dry_run: bool = False,
                  stop: threading.Event | None = None, refresh_days: int = 0,
                  refresh_limit: int = 100, retry_failed: bool = False,
                  budget: RefreshBudget | None = None) -> SellerSummary:
    summary = SellerSummary(source.id)
    try:
        shard = load_shard(data_dir, source.id)
        pending_failures = set(read_run(data_dir, source.id).get("failed_ids", []))
    except ValueError as exc:
        summary.error = str(exc)
        return summary
    budget = budget or RefreshBudget(refresh_limit)
    today = date.fromisoformat(run_ts[:10])
    blocked_streak = 0
    # Independent of blocked_streak above: a run of album-page successes must
    # NOT reset this, since hotlink protection on the thumb CDN is a separate
    # failure mode from album-page blocking and can trip while album pages
    # keep fetching fine.
    thumb_blocked_streak = 0
    thumbs_disabled = False

    def fetch_thumb(url: str, rel: str) -> bool:
        nonlocal thumb_blocked_streak, thumbs_disabled
        if thumbs_disabled:
            return False
        try:
            fetched = client.download(url, data_dir / rel, referer=source.url)
            thumb_blocked_streak = 0
            return fetched
        except BlockedError:
            # thumb blocked: keep album, placeholder shows in viewer. After
            # _THUMB_BLOCKED_STREAK_LIMIT consecutive blocks, stop trying
            # thumbs for the rest of this seller's run - albums keep
            # scraping normally, only the thumb download is skipped.
            thumb_blocked_streak += 1
            if thumb_blocked_streak >= _THUMB_BLOCKED_STREAK_LIMIT:
                thumbs_disabled = True
                summary.thumbs_blocked = True
            return False

    try:
        stubs = _fetch_all_stubs(client, source, data_dir, save_debug=not dry_run, stop=stop)
    except _Stopped:
        summary.error = "interrupted before the listing finished"
        return summary
    except BlockedError as exc:
        summary.error = f"blocked: {exc}"
        return summary
    except (FetchError, LayoutChangeError, EmptySellerError) as exc:
        summary.error = str(exc)
        return summary

    summary.listing_complete = True
    listed = {stub.id for stub in stubs}
    # A complete listing proves which failed albums are no longer available to retry.
    summary.resolved_ids.extend(sorted(pending_failures - listed))
    if refresh_days:
        stubs.sort(key=lambda stub: shard.get(stub.id, {}).get("updated_at", ""))
    # Flushing the shard every so often bounds what an interrupt costs: a
    # big seller at the 1 req/s politeness floor takes tens of minutes, and
    # writing only at the end means Ctrl+C or a crash throws away the whole
    # run. The write is atomic (temp + replace), so a flush can never leave
    # a torn shard for the viewer.
    _FLUSH_EVERY = 100
    scraped_since_flush = 0
    for stub in stubs:
        if stop is not None and stop.is_set():
            summary.error = "interrupted; progress so far is saved"
            break
        old = shard.get(stub.id)
        if retry_failed and stub.id not in pending_failures:
            summary.skipped += 1
            continue
        thumb_rel = f"thumbs/{source.id}/{stub.id}.jpg"
        thumb_path = data_dir / thumb_rel
        # thumb_url names the cover the file on disk holds, and it moves only
        # when a download lands, so a changed cover or a failed download is
        # tried again on the next run. A record from before thumb_url was
        # stored counts as holding the listed cover: its file came from this
        # album.
        held_url = (old or {}).get("thumb_url", stub.thumb_url)
        cover_current = not stub.thumb_url or (
            held_url == stub.thumb_url and thumb_path.exists())
        # Age gate first: a stale listing costs nothing to skip here, but an
        # album page plus a thumb download each if we go on. The id dates the
        # album without a request; a date captured by an earlier scrape wins
        # over the estimate. Records already on disk are left untouched.
        if is_stale(int(stub.id), max_age_days, today,
                    (old or {}).get("date_published", "")):
            summary.skipped_old += 1
            continue
        unchanged = old is not None and old.get("list_title") == stub.title and "price" in old
        refresh = False
        if (old is not None and unchanged and not full and not retry_failed
                and detail_due(old, run_ts, refresh_days)):
            refresh = budget.take()
            if not refresh:
                summary.deferred += 1
        if old is not None and unchanged and not full and not retry_failed and not refresh:
            # Unchanged titles still need a fetch when the on-disk record
            # predates price extraction (no "price" key). Records that already
            # carry a price (including 0, meaning none was found) stay skipped.
            summary.skipped += 1
            # A cover that changed, or failed to arrive on an earlier run
            # (blocked, cut off, invalid), would otherwise never be fetched,
            # since the album itself is unchanged. Its URL is on the list
            # page, so this costs one image request and no album page.
            if (not dry_run and not cover_current
                    and fetch_thumb(stub.thumb_url, thumb_rel)):
                summary.thumbs_refetched += 1
                shard[stub.id] = {**old, "thumb": thumb_rel, "thumb_url": stub.thumb_url,
                                  **cover_facts(thumb_path), "cover_prepared": 1}
            continue
        if dry_run:
            summary.refreshed += int(refresh)
            summary.scraped += 1
            continue
        # Yupoo album pages 404 on the bare form and 200 on the ?uid=1 form
        # (verified live against a real seller: every bare-URL fetch
        # 404'd). The query string is required for both the fetch and the
        # stored record - the stored album_url is the viewer's click-through
        # link, and it must be the working form, which is also the form
        # Yupoo's own hrefs use.
        album_url = f"{source.url}/albums/{stub.id}?uid=1"
        try:
            detail = parse_album_page(client.get(album_url, referer=source.url).text)
            blocked_streak = 0
        except BlockedError as exc:
            blocked_streak += 1
            summary.failed += 1
            summary.failed_ids.append(stub.id)
            if blocked_streak >= _BLOCKED_STREAK_LIMIT:
                summary.error = f"blocked: aborted after {blocked_streak} consecutive ({exc})"
                break
            continue
        except FetchError:
            summary.failed += 1
            summary.failed_ids.append(stub.id)
            continue

        if not detail.title and not detail.description:
            summary.failed += 1
            summary.failed_ids.append(stub.id)
            continue
        title = detail.title or stub.title
        # The list-page title is included because a purchase link sometimes
        # only appears there - a common Yupoo pattern is a clean album-page
        # data-name with the link carried in the list title instead. extract_links
        # dedupes, so overlap with title/detail.description costs nothing.
        links = extract_links(f"{title}\n{stub.title}\n{detail.description}")
        evidence = price_evidence(f"{title} {detail.description}")
        if not evidence:
            evidence = price_evidence(f"{title} {stub.title} {detail.description}")
        if evidence:
            evidence["field"] = next((name for name, text in
                (("title", title), ("listing title", stub.title), ("description", detail.description))
                if evidence["token"] in text), "album text")
        price = evidence.get("amount", 0)
        # An album page is re-fetched for many reasons (a changed title, a
        # price backfill) that say nothing about its cover. Downloading the
        # cover again anyway doubles the requests of every such run.
        cover_downloaded = False
        if stub.thumb_url and (full or not cover_current):
            cover_current = fetch_thumb(stub.thumb_url, thumb_rel)
            cover_downloaded = cover_current
        record = {
            "id": stub.id,
            "seller": source.id,
            "title": title,
            "list_title": stub.title,
            # No path when Yupoo has no cover for the album at all (parse.py
            # reports its placeholder asset as no thumb). Recording one
            # anyway points the viewer's <img> at a file that can never
            # exist, which is a 404 per card on every scroll-in; the titled
            # colour block is the intended fallback. A cover that exists but
            # whose download was blocked keeps its path - the file can still
            # arrive on a later run.
            "thumb": thumb_rel if stub.thumb_url else "",
            "thumb_url": stub.thumb_url if cover_current else (old or {}).get("thumb_url", ""),
            "album_url": album_url,
            "links": [{"platform": link.platform, "url": link.url} for link in links],
            "category": detail.category,
            # Bounded, and present (possibly empty) whenever the album page
            # was read, so the viewer can tell "no description on Yupoo"
            # from "not fetched since descriptions were kept".
            "description": detail.description[:DESCRIPTION_LIMIT],
            "price": price,
            "price_evidence": evidence,
            "photo_count": detail.photo_count or stub.photo_count,
            "date_published": detail.date_published,
            "date_modified": detail.date_modified,
            "first_seen": "",
            "updated_at": "",
        }
        if len(detail.description) > DESCRIPTION_LIMIT:
            record["description_truncated"] = True
        if stub.thumb_url and cover_current and thumb_path.is_file():
            record.update(cover_facts(thumb_path))
            if cover_downloaded:
                record["cover_prepared"] = 1
        shard[stub.id] = merge_album(old, record, run_ts)
        summary.scraped += 1
        summary.refreshed += int(refresh)
        summary.resolved_ids.append(stub.id)
        if detail.date_published or detail.date_modified:
            summary.dated += 1
        scraped_since_flush += 1
        if scraped_since_flush >= _FLUSH_EVERY:
            write_shard(data_dir, source.id, shard)
            scraped_since_flush = 0

    # A seller whose list pages parse fine but whose album-page fetches all
    # fail must not report clean - each failure alone just increments
    # `failed`, so a 100% failure rate would otherwise leave summary.error
    # empty and the CLI would exit 0. Only trip this when at least one album
    # fetch was actually attempted (skips don't count) and nothing scraped;
    # partial success (scraped > 0) is left alone.
    attempted = len(stubs) - summary.skipped
    if attempted > 0 and summary.scraped == 0 and summary.failed > 0 and not summary.error:
        summary.error = f"all {summary.failed} album fetches failed"

    if not dry_run:
        _stamp_listing(shard, listed, run_ts)
        write_shard(data_dir, source.id, shard)
    return summary


def _stamp_listing(shard: dict[str, dict], listed: set[str], run_ts: str) -> None:
    """Record which albums the seller's complete listing still carries.

    last_seen tells the viewer when this seller was last listed, which is
    what "new" is measured against. An album missing from a complete listing
    has been deleted on Yupoo: it keeps its record (a favourite must not
    vanish) but gains gone_since, and loses it again if it is relisted.
    """
    for album_id, album in shard.items():
        if album_id in listed:
            album["last_seen"] = run_ts
            if album.pop("gone_since", None):
                record_change(album, "availability", "gone", "listed", run_ts)
        else:
            if not album.get("gone_since"):
                record_change(album, "availability", "listed", "gone", run_ts)
                album["gone_since"] = run_ts


def scrape_all(sources: dict[str, Source], data_dir: Path, run_ts: str,
               full: bool = False, workers: int = 4, client_factory=None,
               only: set[str] | None = None, rps: float = 8.0,
               max_age_days: int = 0, dry_run: bool = False,
               on_done: Callable[[SellerSummary], None] | None = None,
               refresh_days: int = 0, refresh_limit: int = 100, retry_failed: bool = False,
               ) -> list[SellerSummary]:
    """Scrape every seller, then rewrite the index from the shards on disk.

    on_done is called with each seller's summary as soon as that seller
    finishes, so a run of several hours reports as it goes. Ctrl+C stops the
    run gracefully: queued sellers are dropped, running ones save what they
    have at their next album, the index is written, and KeyboardInterrupt is
    raised again for the caller.
    """
    budget = RefreshBudget(refresh_limit)
    threads: list[threading.Thread] = []
    with nullcontext() if dry_run else catalogue_lock(data_dir, threads):
        if client_factory is None:
            # The shared thumbnail CDN host would otherwise serialise the whole
            # run at 1 req/s; it runs at the global cap instead. Per-host 1 req/s
            # for seller hosts is a politeness floor and stays fixed - only the
            # global cap is operator-tunable.
            limiter = RateLimiter(global_rps=rps, host_rps=1.0,
                                  host_overrides={"photo.yupoo.com": rps})

            def client_factory() -> Client:
                return Client(limiter)

        sellers = sorted(
            (src for src in sources.values() if src.kind == "yupoo"), key=lambda s: s.id
        )
        shops = sorted(
            (src for src in sources.values() if src.kind == "shop"), key=lambda s: s.id
        )
        # skip = true in sources.toml leaves a seller out of a normal run, but
        # naming it explicitly with --seller still scrapes it: an explicit
        # request should not be silently ignored.
        if only is not None:
            to_scrape = [src for src in sellers if src.id in only]
        else:
            to_scrape = [src for src in sellers if not src.skip]

        stop = threading.Event()

        def run_one(source: Source) -> SellerSummary:
            return scrape_seller(client_factory(), source, data_dir, run_ts, full=full,
                                 max_age_days=max_age_days, dry_run=dry_run, stop=stop,
                                 refresh_days=refresh_days, refresh_limit=refresh_limit,
                                 retry_failed=retry_failed, budget=budget)

        todo: queue.SimpleQueue[Source] = queue.SimpleQueue()
        for source in to_scrape:
            todo.put(source)
        results: queue.SimpleQueue[SellerSummary] = queue.SimpleQueue()

        def worker() -> None:
            # A set stop flag leaves queued sellers unstarted.
            while not stop.is_set():
                try:
                    source = todo.get_nowait()
                except queue.Empty:
                    return
                try:
                    results.put(run_one(source))
                except Exception as exc:
                    # One seller's unexpected failure is that seller's error, as
                    # a corrupt shard is: the rest of the run carries on. It is a
                    # bug, so its traceback goes to stderr for diagnosis.
                    print(f"{source.id}: unexpected error\n{traceback.format_exc()}",
                          file=sys.stderr, end="", flush=True)
                    results.put(SellerSummary(source.id, error=f"unexpected error: {exc!r}"))

        # Daemon threads, not a ThreadPoolExecutor: the interpreter joins pool
        # threads at exit, so a seller stuck in a slow request would hold the
        # process open after the operator abandoned it with a second Ctrl+C.
        # Every shard write is atomic, so a thread cut off at exit leaves at
        # most a stray .tmp file.
        threads.extend(threading.Thread(target=worker, daemon=True)
                       for _ in range(min(workers, len(to_scrape))))
        for thread in threads:
            thread.start()

        summaries: list[SellerSummary] = []

        def drain() -> None:
            # A timed get wakes the main thread twice a second, so Ctrl+C is
            # handled promptly rather than whenever the next seller finishes.
            while True:
                idle = not any(thread.is_alive() for thread in threads)
                try:
                    # Once every worker has exited, all results are queued.
                    item = results.get(block=not idle, timeout=0.5)
                except queue.Empty:
                    if idle:
                        return
                    continue
                summaries.append(item)
                if not dry_run:
                    write_run(data_dir, item.seller, run_ts, asdict(item))
                if on_done is not None:
                    on_done(item)

        interrupted = False
        try:
            drain()
        except KeyboardInterrupt:
            interrupted = True
            stop.set()
            print("stopping: running sellers save their progress, "
                  "Ctrl+C again to abandon them", file=sys.stderr, flush=True)
            try:
                drain()
            except KeyboardInterrupt:
                pass  # a second press abandons the running sellers, not the index
        finally:
            # Sellers are still running here only after a second Ctrl+C or an
            # error raised by on_done; the stop flag ends each at its next album.
            stop.set()
        summaries.sort(key=lambda s: s.seller)

        if not dry_run:
            _write_index(data_dir, sellers, shops, run_ts)
        if interrupted:
            raise KeyboardInterrupt
        return summaries


def _write_index(data_dir: Path, sellers: list[Source], shops: list[Source],
                 run_ts: str) -> None:
    # Index covers ALL yupoo sources, not just the scraped subset: a scoped
    # --seller run must never drop other sellers from the viewer manifest.
    # Sellers marked skip are the exception - they have no shard and never
    # will, so listing them would make the viewer report a deliberate
    # exclusion as an unavailable seller on every load.
    seller_meta = []
    for source in sellers:
        try:
            shard = load_shard(data_dir, source.id)
        except ValueError:
            # Listed anyway, so the viewer's fetch of it fails and its load
            # note names the seller. Leaving it out would make the seller
            # vanish with nothing said. The per-seller error summary names
            # the corrupt file on the terminal.
            album_count = 0
        else:
            if source.skip and not shard:
                continue
            album_count = sum(1 for album in shard.values() if not album.get("gone_since"))
        seller_meta.append({
            "id": source.id, "name": source.name, "url": source.url,
            "album_count": album_count, "shard": f"catalog/{source.id}.json",
            "run": read_run(data_dir, source.id),
        })
    shop_meta = [
        {"id": s.id, "platform": s.platform, "name": s.name, "url": s.url} for s in shops
    ]
    write_index(data_dir, seller_meta, shop_meta, run_ts)
