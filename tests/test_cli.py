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


def test_scrape_reports_each_diagnostic_and_exits_zero(tmp_path: Path, capsys, monkeypatch):
    summary = SellerSummary("sellerx", scraped=4, skipped=2, failed=1,
                            failed_ids=["9"], thumbs_blocked=True, dated=0, skipped_old=7)
    monkeypatch.setattr("lamp.scrape.scrape_all", lambda *a, **k: [summary])
    rc = main(["--sources", str(_sources_file(tmp_path)), "--data", str(tmp_path / "d"),
               "scrape"])
    out = capsys.readouterr().out
    assert rc == 0
    assert "4 scraped, 2 skipped, 1 failed" in out
    assert "7 too old" in out
    assert "failed albums: 9" in out
    assert "thumbs blocked" in out
    # a seller that scraped cleanly but produced no dates signals a markup change
    assert "no publish dates found" in out
    assert "1 sellers, 0 with errors" in out


def test_scrape_exits_nonzero_when_a_seller_errors(tmp_path: Path, capsys, monkeypatch):
    summary = SellerSummary("sellerx", error="blocked: HTTP 429")
    monkeypatch.setattr("lamp.scrape.scrape_all", lambda *a, **k: [summary])
    rc = main(["--sources", str(_sources_file(tmp_path)), "--data", str(tmp_path / "d"),
               "scrape"])
    assert rc == 1
    assert "ERROR: blocked: HTTP 429" in capsys.readouterr().out


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
          "scrape", "--max-age-days", "180", "--rps", "20", "--workers", "16"])
    assert seen["max_age_days"] == 180
    assert seen["rps"] == 20.0
    assert seen["workers"] == 16
