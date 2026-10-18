//! Ported from tests/test_locking.py.
mod common;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lamp::errors::Error;
use lamp::locking::CatalogueLock;

/// Runs only as a child process: holds the lock until killed.
#[test]
fn hold_lock_helper() {
    let Ok(dir) = std::env::var("LAMP_HOLD_LOCK") else {
        return;
    };
    let _lock = CatalogueLock::acquire(std::path::Path::new(&dir)).unwrap();
    std::fs::write(std::path::Path::new(&dir).join("ready"), b"1").unwrap();
    std::thread::sleep(Duration::from_secs(20));
}

#[test]
fn lock_excludes_processes_and_recovers_after_a_crash() {
    let dir = common::tmp();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "hold_lock_helper", "--nocapture"])
        .env("LAMP_HOLD_LOCK", dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !dir.path().join("ready").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "helper never locked"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(matches!(
        CatalogueLock::acquire(dir.path()),
        Err(Error::Busy(_))
    ));
    let (code, _, err) = common::lamp(&["--data", &dir.path().to_string_lossy(), "snapshot"], None);
    assert_eq!(code, 1);
    assert!(err.contains("catalogue is already in use"), "{err}");
    drop(CatalogueLock::acquire(&dir.path().join("separate")).unwrap());
    child.kill().unwrap();
    child.wait().unwrap();
    drop(CatalogueLock::acquire(dir.path()).unwrap());
}

#[test]
fn the_lock_is_released_when_dropped_and_the_file_stays() {
    let dir = common::tmp();
    let lock = CatalogueLock::acquire(dir.path()).unwrap();
    assert!(matches!(
        CatalogueLock::acquire(dir.path()),
        Err(Error::Busy(_))
    ));
    drop(lock);
    drop(CatalogueLock::acquire(dir.path()).unwrap());
    assert!(dir.path().join(".lamp.lock").exists());
}
