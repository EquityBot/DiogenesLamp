"""Process locks for a catalogue's complete read/modify/write operation."""

from __future__ import annotations

import os
import sys
import threading
from collections.abc import Iterator, Sequence
from contextlib import contextmanager
from pathlib import Path


class CatalogueBusyError(RuntimeError):
    """Another process is modifying this catalogue."""


@contextmanager
def catalogue_lock(data_dir: Path, workers: Sequence[threading.Thread] = ()) -> Iterator[None]:
    """Hold an OS lock, including workers abandoned by a second interrupt.

    The lock file stays in place: unlinking it would let another process lock
    a different inode. The operating system releases the lock after a crash.
    ponytail: writers share one lock per data directory. Seller-level locks
    would also need a coordinated manifest writer if concurrent runs are needed.
    """
    root = data_dir.resolve()
    root.mkdir(parents=True, exist_ok=True)
    handle = (root / ".lamp.lock").open("a+b")
    try:
        if sys.platform == "win32":
            import msvcrt

            handle.seek(0, os.SEEK_END)
            if handle.tell() == 0:
                handle.write(b"\0")
                handle.flush()
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl

            fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError as exc:
        handle.close()
        raise CatalogueBusyError(f"catalogue is already in use: {root}") from exc

    def release() -> None:
        try:
            if sys.platform == "win32":
                import msvcrt

                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl

                fcntl.flock(handle.fileno(), fcntl.LOCK_UN)
        finally:
            handle.close()

    try:
        yield
    finally:
        alive = [worker for worker in workers if worker.is_alive()]
        if alive:
            # A second Ctrl+C returns promptly, but those workers may still
            # save progress. Keep their lock until they finish or the process exits.
            def finish() -> None:
                for worker in alive:
                    worker.join()
                release()

            threading.Thread(target=finish, daemon=True).start()
        else:
            release()
