"""lamp command line interface."""

from __future__ import annotations

import argparse
import math
import sys
from datetime import UTC
from pathlib import Path

from lamp.ingest import load_sources, merge_sources, parse_dump, write_sources


def _positive_int(text: str) -> int:
    value = int(text)  # argparse's type= surfaces ValueError as a usage error
    if value < 1:
        raise argparse.ArgumentTypeError("must be 1 or greater")
    return value


def _positive_float(text: str) -> float:
    value = float(text)
    # 0 would divide-by-zero deep inside the rate limiter; -h would make
    # every wait() negative, unthrottling the scrape entirely. NaN and inf
    # need naming separately: float("nan") <= 0 is False, so NaN slips a
    # bare comparison and then poisons every gap arithmetic downstream,
    # and inf collapses the global gap to zero - both quietly discard the
    # politeness cap this flag exists to set.
    if not math.isfinite(value) or value <= 0:
        raise argparse.ArgumentTypeError("must be a finite number greater than 0")
    return value


def _port(text: str) -> int:
    value = int(text)
    if not 1 <= value <= 65535:
        raise argparse.ArgumentTypeError("port must be between 1 and 65535")
    return value


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="lamp", description=__doc__)
    parser.add_argument("--sources", type=Path, default=Path("sources.toml"))
    parser.add_argument("--data", type=Path, default=Path("data"))
    sub = parser.add_subparsers(dest="command", required=True)

    p_import = sub.add_parser("import", help="import a raw link dump into sources.toml")
    p_import.add_argument("dump", type=Path)

    p_scrape = sub.add_parser("scrape", help="scrape Yupoo sellers into data/")
    p_scrape.add_argument("--seller", help="scrape only this seller id")
    p_scrape.add_argument("--full", action="store_true", help="re-fetch unchanged albums")
    p_scrape.add_argument("--workers", type=_positive_int, default=8)
    p_scrape.add_argument("--rps", type=_positive_float, default=8.0,
                          help="global request rate cap")
    p_scrape.add_argument("--max-age-days", type=int, default=0, metavar="N",
                          help="skip albums posted more than N days ago "
                               "(0 = no age limit); stock that old is usually gone")

    p_serve = sub.add_parser("serve", help="serve the local viewer")
    p_serve.add_argument("--port", type=_port, default=8000)
    p_serve.add_argument("--no-browser", action="store_true")
    return parser


def _sniff_encoding(blob: bytes) -> str:
    """Text encoding of a raw dump, by byte order mark.

    Excel exports CSVs as UTF-16 with a BOM on many locales, so assuming
    UTF-8 outright kills the import on a file the user considers ordinary.
    """
    for bom, encoding in ((b"\xff\xfe", "utf-16-le"), (b"\xfe\xff", "utf-16-be"),
                          (b"\xef\xbb\xbf", "utf-8-sig")):
        if blob.startswith(bom):
            return encoding
    return "utf-8"


def _cmd_import(args: argparse.Namespace) -> int:
    if not args.dump.exists():
        print(f"error: dump file not found: {args.dump}", file=sys.stderr)
        return 1
    blob = args.dump.read_bytes()
    encoding = _sniff_encoding(blob)
    try:
        text = blob.decode(encoding)
    except UnicodeDecodeError as exc:
        # A dump saved in a legacy code page, or a UTF-16 file with a stray
        # byte, reaches here. Name the encoding tried and what to do rather
        # than showing the raw decoder traceback.
        print(
            f"error: could not read {args.dump} as {encoding} ({exc.reason} at "
            f"byte {exc.start}); re-save it as UTF-8 and try again",
            file=sys.stderr
        )
        return 1
    raws = parse_dump(text)
    existing = load_sources(args.sources)
    merged, report = merge_sources(existing, raws)
    write_sources(args.sources, merged)
    platforms = ""
    if report.shops_by_platform:
        detail = ", ".join(f"{k} {v}" for k, v in sorted(report.shops_by_platform.items()))
        platforms = f" ({detail})"
    print(
        f"{report.rows} rows read, {report.invalid} invalid\n"
        f"yupoo sellers: {report.yupoo_new} new, {report.yupoo_known} known\n"
        f"shops: {report.shops_new} new, {report.shops_known} known{platforms}\n"
        f"wrote {args.sources}"
    )
    return 0


def _cmd_scrape(args: argparse.Namespace) -> int:
    from datetime import datetime

    from lamp.scrape import scrape_all

    sources = load_sources(args.sources)
    if not any(src.kind == "yupoo" for src in sources.values()):
        print("error: no sources - run 'lamp import' first", file=sys.stderr)
        return 1
    only = None
    if args.seller:
        if f"yupoo:{args.seller}" not in sources:
            print(f"error: unknown seller: {args.seller}", file=sys.stderr)
            return 1
        only = {args.seller}
    run_ts = datetime.now(UTC).astimezone().isoformat(timespec="seconds")
    summaries = scrape_all(sources, args.data, run_ts,
                           full=args.full, workers=args.workers, only=only, rps=args.rps,
                           max_age_days=args.max_age_days)
    errored = [s for s in summaries if s.error]
    for s in summaries:
        line = f"{s.seller}: {s.scraped} scraped, {s.skipped} skipped, {s.failed} failed"
        if s.skipped_old:
            line += f", {s.skipped_old} too old"
        if s.failed_ids:
            shown = ", ".join(s.failed_ids[:5])
            more = ", ..." if len(s.failed_ids) > 5 else ""
            line += f" (failed albums: {shown}{more})"
        if s.thumbs_blocked:
            line += " (thumbs blocked - hotlink protection?)"
        # Dates come from JSON-LD on the album page. Scraping every album
        # cleanly yet extracting no date at all is the signature of a Yupoo
        # markup change, and it is otherwise invisible: the viewer just
        # falls back to id-based age estimates forever.
        if s.scraped and not s.dated:
            line += " (no publish dates found - Yupoo markup change?)"
        if s.error:
            line += f"  ERROR: {s.error}"
        print(line)
    # Name the skipped sellers rather than quietly reporting a smaller total:
    # a stray skip = true would otherwise be invisible in the summary.
    skipped = sorted(
        src.id for src in sources.values()
        if src.kind == "yupoo" and src.skip and (only is None or src.id not in only)
    )
    tail = f", {len(skipped)} skipped ({', '.join(skipped)})" if skipped else ""
    print(f"{len(summaries)} sellers, {len(errored)} with errors{tail}")
    return 1 if errored else 0


def main(argv: list[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)
    if args.command == "import":
        return _cmd_import(args)
    if args.command == "scrape":
        return _cmd_scrape(args)
    if args.command == "serve":
        from lamp.serve import serve

        if not (args.data / "index.json").exists():
            # The viewer will load but sit on an error message: say why on
            # the terminal, where the operator actually is.
            print(
                f"note: no index.json under {args.data} - the viewer will show "
                f"an empty catalog until 'lamp scrape' writes one",
                file=sys.stderr
            )
        viewer_dir = Path(__file__).resolve().parents[2] / "viewer"
        serve(viewer_dir, args.data, port=args.port, open_browser=not args.no_browser)
        return 0
    raise AssertionError("unreachable")


if __name__ == "__main__":
    raise SystemExit(main())
