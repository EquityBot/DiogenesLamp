from pathlib import Path

from lamp.cli import main
from lamp.scrape import SellerSummary


def test_import_creates_sources(tmp_path: Path, capsys):
    dump = tmp_path / "links.csv"
    dump.write_text(
        "1\thttps://sellerx.x.yupoo.com/\n2\thttps://weidian.com/?userid=7\n3\tN/A\n",
        encoding="utf-8",
    )
    sources = tmp_path / "sources.toml"
    rc = main(["--sources", str(sources), "import", str(dump)])
    assert rc == 0
    text = sources.read_text(encoding="utf-8")
    assert "[yupoo.sellerx]" in text
    assert "[shops.weidian-7]" in text
    out = capsys.readouterr().out
    assert "3 rows" in out and "1 invalid" in out
    assert "yupoo sellers: 1 new, 0 known" in out
    assert "shops: 1 new, 0 known" in out


def test_import_missing_file_errors(tmp_path: Path, capsys):
    rc = main(["--sources", str(tmp_path / "s.toml"), "import", str(tmp_path / "nope.csv")])
    assert rc == 1
    assert "not found" in capsys.readouterr().err


def test_import_accepts_utf16_excel_exports(tmp_path: Path):
    """Excel's default CSV export is UTF-16 with a BOM on many locales;
    importing one used to die on a raw UnicodeDecodeError."""
    dump = tmp_path / "dump.csv"
    dump.write_bytes("1\thttps://sellerx.x.yupoo.com/\n".encode("utf-16"))
    rc = main(["--sources", str(tmp_path / "s.toml"), "import", str(dump)])
    assert rc == 0
    text = (tmp_path / "s.toml").read_text(encoding="utf-8")
    assert "[yupoo.sellerx]" in text


def test_import_reports_an_undecodable_dump_instead_of_crashing(tmp_path: Path, capsys):
    """A dump saved in a legacy code page has no BOM to sniff and is not
    UTF-8 either; say so and name the fix rather than showing the decoder's
    traceback."""
    dump = tmp_path / "dump.csv"
    dump.write_bytes(b"https://ok.x.yupoo.com/\n\x81\x9d not utf-8")
    rc = main(["--sources", str(tmp_path / "s.toml"), "import", str(dump)])
    assert rc == 1
    err = capsys.readouterr().err
    assert "could not read" in err and "utf-8" in err
    assert "re-save it as UTF-8" in err
    assert not (tmp_path / "s.toml").exists()  # nothing half-written


def test_scrape_requires_sources(tmp_path: Path, capsys):
    rc = main(["--sources", str(tmp_path / "none.toml"), "--data", str(tmp_path / "d"), "scrape"])
    assert rc == 1
    assert "no sources" in capsys.readouterr().err


def _sources_file(tmp_path: Path) -> Path:
    path = tmp_path / "sources.toml"
    path.write_text(
        '[yupoo.sellerx]\nurl = "https://sellerx.x.yupoo.com"\nname = "sellerx"\n',
        encoding="utf-8",
    )
    return path


def _fake_scrape_all(summary):
    """Stands in for scrape_all, reporting through on_done as the real one does."""
    def fake(*_args, on_done=None, **_kwargs):
        if on_done is not None:
            on_done(summary)
        return [summary]
    return fake


def test_scrape_reports_each_diagnostic_and_exits_zero(tmp_path: Path, capsys, monkeypatch):
    summary = SellerSummary("sellerx", scraped=4, skipped=2, failed=1,
                            failed_ids=["9"], thumbs_blocked=True, dated=0, skipped_old=7,
                            thumbs_refetched=3)
    monkeypatch.setattr("lamp.scrape.scrape_all", _fake_scrape_all(summary))
    rc = main(["--sources", str(_sources_file(tmp_path)), "--data", str(tmp_path / "d"),
               "scrape"])
    out = capsys.readouterr().out
    assert rc == 0
    assert "[1/1] sellerx: 4 scraped, 2 skipped, 1 failed" in out
    assert "7 too old" in out
    assert "3 missing covers fetched" in out
    assert "failed albums: 9" in out
    assert "thumbs blocked" in out
    # a seller that scraped cleanly but produced no dates signals a markup change
    assert "no publish dates found" in out
    assert "1 sellers, 0 with errors" in out


def test_scrape_exits_nonzero_when_a_seller_errors(tmp_path: Path, capsys, monkeypatch):
    summary = SellerSummary("sellerx", error="blocked: HTTP 429")
    monkeypatch.setattr("lamp.scrape.scrape_all", _fake_scrape_all(summary))
    rc = main(["--sources", str(_sources_file(tmp_path)), "--data", str(tmp_path / "d"),
               "scrape"])
    assert rc == 1
    assert "ERROR: blocked: HTTP 429" in capsys.readouterr().out


def test_scrape_interrupted_exits_130_and_says_what_was_kept(tmp_path: Path, capsys,
                                                           monkeypatch):
    def interrupted(*_args, **_kwargs):
        raise KeyboardInterrupt
    monkeypatch.setattr("lamp.scrape.scrape_all", interrupted)
    rc = main(["--sources", str(_sources_file(tmp_path)), "--data", str(tmp_path / "d"),
               "scrape"])
    assert rc == 130
    assert "interrupted after 0 of 1 sellers; finished work is saved" in capsys.readouterr().err


def test_scrape_rejects_an_unknown_seller(tmp_path: Path, capsys):
    rc = main(["--sources", str(_sources_file(tmp_path)), "--data", str(tmp_path / "d"),
               "scrape", "--seller", "nosuchseller"])
    assert rc == 1
    assert "unknown seller" in capsys.readouterr().err


def test_scrape_passes_max_age_days_through(tmp_path: Path, monkeypatch):
    seen = {}
    def fake_scrape_all(sources, data_dir, run_ts, **kwargs):
        seen.update(kwargs)
        return []
    monkeypatch.setattr("lamp.scrape.scrape_all", fake_scrape_all)
    main(["--sources", str(_sources_file(tmp_path)), "--data", str(tmp_path / "d"),
          "scrape", "--max-age-days", "180", "--rps", "20", "--workers", "16", "--dry-run"])
    assert seen["max_age_days"] == 180
    assert seen["rps"] == 20.0
    assert seen["workers"] == 16
    assert seen["dry_run"] is True


def test_serve_warns_when_no_index_json(tmp_path: Path, capsys, monkeypatch):
    """`lamp serve` over a data dir with no index.json must say so on the
    terminal, not leave the operator guessing why the viewer is empty."""
    def fake_serve(viewer_dir, data_dir, port=8000, open_browser=True):
        pass
    monkeypatch.setattr("lamp.serve.serve", fake_serve)
    rc = main(["--data", str(tmp_path), "serve", "--no-browser"])
    assert rc == 0
    assert "no index.json" in capsys.readouterr().err


def test_serve_refuses_when_the_viewer_tree_is_missing(tmp_path: Path, capsys, monkeypatch):
    """The viewer is served straight out of the source tree, so only an
    editable install has one. After a plain `pip install .` the path points
    inside site-packages at nothing, and the server used to start, open a
    browser and answer 404 to every request without saying a word."""
    served = []
    # Stand the package somewhere with no viewer/ beside it, which is exactly
    # what a non-editable install looks like: the path is derived from the
    # module's own __file__.
    monkeypatch.setattr("lamp.cli.__file__", str(tmp_path / "site" / "lamp" / "cli.py"))
    monkeypatch.setattr("lamp.serve.serve", lambda *a, **k: served.append(a))
    (tmp_path / "index.json").write_text("{}", encoding="utf-8")
    rc = main(["--data", str(tmp_path), "serve", "--no-browser"])
    assert rc == 1
    assert served == []  # nothing bound a port or opened a browser
    err = capsys.readouterr().err
    assert "viewer files not found" in err
    assert "pip install -e" in err


def test_nonsense_numeric_args_are_usage_errors_not_tracebacks(capsys):
    """--workers 0, --rps 0 and --port 99999 used to crash deep inside the
    run (ZeroDivisionError, ValueError) with a raw traceback. argparse must
    reject them up front with a usage error (exit 2) instead."""
    import pytest

    for argv in (
        ["scrape", "--workers", "0"],
        ["scrape", "--rps", "0"],
        ["scrape", "--rps", "-4"],
        # NaN and inf are not caught by a bare `> 0` check; see
        # _positive_float for what they do to the limiter.
        ["scrape", "--rps", "nan"],
        ["scrape", "--rps", "inf"],
        ["serve", "--port", "0"],
        ["serve", "--port", "99999"],
    ):
        with pytest.raises(SystemExit) as err:
            main(argv)
        assert err.value.code == 2
    # none of them got as far as touching the network or disk: every
    # rejection was printed as usage text on stderr.
    assert "error" in capsys.readouterr().err
