import multiprocessing
import threading
from pathlib import Path

import pytest

from lamp.locking import CatalogueBusyError, catalogue_lock
from lamp.scrape import scrape_all


def hold_lock(path, ready, finish):
    with catalogue_lock(Path(path)):
        ready.set()
        finish.wait(20)


def test_lock_excludes_processes_and_recovers_after_crash(tmp_path):
    context = multiprocessing.get_context("spawn")
    ready, finish = context.Event(), context.Event()
    process = context.Process(target=hold_lock, args=(str(tmp_path), ready, finish))
    process.start()
    try:
        assert ready.wait(10)
        with pytest.raises(CatalogueBusyError):
            with catalogue_lock(tmp_path):
                pytest.fail("overlapping writers entered")
        with catalogue_lock(tmp_path / "separate"):
            pass
        process.terminate()
        process.join(10)
        assert not process.is_alive()
        with catalogue_lock(tmp_path):
            pass
    finally:
        if process.is_alive():
            process.terminate()
        process.join(10)


def test_lock_stays_with_worker_after_interrupt(tmp_path):
    finish = threading.Event()
    worker = threading.Thread(target=lambda: finish.wait(10), daemon=True)
    worker.start()
    try:
        with catalogue_lock(tmp_path, [worker]):
            pass
        with pytest.raises(CatalogueBusyError):
            with catalogue_lock(tmp_path):
                pytest.fail("lock released while worker could still write")
    finally:
        finish.set()
        worker.join(10)


def test_scrape_lock_precedes_any_fetch(tmp_path):
    with catalogue_lock(tmp_path):
        with pytest.raises(CatalogueBusyError):
            scrape_all({}, tmp_path, "2026-09-26T00:00:00+00:00")


def test_dry_run_does_not_create_lock_or_data(tmp_path):
    target = tmp_path / "absent"
    assert scrape_all({}, target, "2026-09-26T00:00:00+00:00", dry_run=True) == []
    assert not target.exists()
