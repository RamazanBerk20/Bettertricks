use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;

use uuid::Uuid;

use crate::{AppPaths, BettertricksError, Result};

/// Cross-process "one mutating operation per prefix" lock.
///
/// Uses Linux open-file-description locks (`F_OFD_SETLK`): they conflict between processes
/// *and* between separate opens inside one process, are released by the kernel when the
/// holder dies, and can be probed without being taken (`F_OFD_GETLK`).
#[derive(Debug, Clone)]
pub struct PrefixLocks {
    root: PathBuf,
    /// Set when no candidate directory supports OFD locks; every acquire then fails with it.
    unsupported: Option<String>,
}

/// What an instant probe of a prefix lock can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockProbe {
    Free,
    Held,
    /// No lock file, or the probe failed: nothing can be concluded about a running operation.
    Unknown,
}

/// Held for as long as the prefix is being mutated; dropping it releases the lock.
#[derive(Debug)]
pub struct PrefixLock {
    _file: File,
}

impl PrefixLocks {
    /// Uses `<state>/locks`, or `$XDG_RUNTIME_DIR/bettertricks/locks` when the state directory
    /// sits on a filesystem without OFD locks (some FUSE, 9p or vboxsf mounts). Every process
    /// of the same user makes the same choice, so they still share one lock table.
    pub fn new(paths: &AppPaths) -> Self {
        let mut candidates = vec![paths.state.join("locks")];
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
            candidates.push(PathBuf::from(runtime).join("bettertricks/locks"));
        }
        for root in &candidates {
            if locks_work(root) {
                return Self {
                    root: root.clone(),
                    unsupported: None,
                };
            }
        }
        Self {
            root: candidates[0].clone(),
            unsupported: Some(format!(
                "Bettertricks needs file locks to keep two tasks from changing one prefix, but {} does not support them. Move the state directory (XDG_STATE_HOME) to a local disk, or set XDG_RUNTIME_DIR.",
                candidates[0].display()
            )),
        }
    }

    fn path(&self, prefix_id: Uuid) -> PathBuf {
        self.root.join(format!("{prefix_id}.lock"))
    }

    fn open(&self, prefix_id: Uuid) -> Result<File> {
        std::fs::create_dir_all(&self.root)?;
        Ok(OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.path(prefix_id))?)
    }

    /// Takes the lock or fails with [`BettertricksError::Busy`] after at most
    /// `RELEASE_GRACE`. Blocking: call it from a blocking thread in async code.
    pub fn acquire(&self, prefix_id: Uuid, holder: &str) -> Result<PrefixLock> {
        if let Some(message) = &self.unsupported {
            return Err(BettertricksError::Unsupported(message.clone()));
        }
        let mut file = self.open(prefix_id)?;
        if !acquire_with_grace(&file)? {
            let mut current = String::new();
            let _ = file.read_to_string(&mut current);
            let current = current.trim();
            return Err(BettertricksError::Busy(if current.is_empty() {
                "another Bettertricks task is changing this prefix".into()
            } else {
                current.to_owned()
            }));
        }
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        let _ = write!(file, "{holder} (pid {})", std::process::id());
        Ok(PrefixLock { _file: file })
    }

    /// Instant, non-blocking probe. A just-released lock may still read `Held` for a few
    /// milliseconds (see `RELEASE_GRACE`), so callers treat `Held` as "skip for now", never as
    /// proof of a live operation; only `Free` permits conclusions.
    pub fn probe(&self, prefix_id: Uuid) -> LockProbe {
        if self.unsupported.is_some() || !self.path(prefix_id).exists() {
            return LockProbe::Unknown;
        }
        match self
            .open(prefix_id)
            .and_then(|file| ofd_lock(&file, libc::F_OFD_GETLK, libc::F_WRLCK))
        {
            Ok(true) => LockProbe::Free,
            Ok(false) => LockProbe::Held,
            Err(_) => LockProbe::Unknown,
        }
    }

    /// For display: whether the prefix currently looks busy.
    pub fn is_held(&self, prefix_id: Uuid) -> bool {
        self.probe(prefix_id) == LockProbe::Held
    }
}

fn locks_work(root: &std::path::Path) -> bool {
    let probe = std::fs::create_dir_all(root).and_then(|_| {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(".probe.lock"))
    });
    probe.is_ok_and(|file| ofd_lock(&file, libc::F_OFD_GETLK, libc::F_WRLCK).is_ok())
}

/// A released lock can look held for a few milliseconds: a process forked by another thread
/// shares every open file description (including this lock's) until it reaches `exec`, which
/// closes the O_CLOEXEC copy. Retry briefly so that window never reads as "busy"; a real holder
/// still fails fast.
const RELEASE_GRACE: std::time::Duration = std::time::Duration::from_millis(250);

fn acquire_with_grace(file: &File) -> Result<bool> {
    let deadline = std::time::Instant::now() + RELEASE_GRACE;
    loop {
        if ofd_lock(file, libc::F_OFD_SETLK, libc::F_WRLCK)? {
            return Ok(true);
        }
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Returns `Ok(true)` when the lock was taken (SETLK) or would be free (GETLK).
fn ofd_lock(file: &File, command: libc::c_int, kind: libc::c_int) -> Result<bool> {
    // SAFETY: `flock` is plain data; zero is a valid initial value and `l_pid` must be 0 for OFD locks.
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = kind as libc::c_short;
    lock.l_whence = libc::SEEK_SET as libc::c_short;
    // SAFETY: the descriptor is owned by `file` and `lock` outlives the call.
    let result = unsafe { libc::fcntl(file.as_raw_fd(), command, &mut lock) };
    if result == -1 {
        let error = std::io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(libc::EAGAIN) | Some(libc::EACCES) => Ok(false),
            Some(libc::ENOLCK | libc::EOPNOTSUPP | libc::EINVAL | libc::ENOSYS) => {
                Err(BettertricksError::Unsupported(format!(
                    "file locks are not supported here: {error}"
                )))
            }
            _ => Err(error.into()),
        };
    }
    if command == libc::F_OFD_GETLK {
        return Ok(lock.l_type == libc::F_UNLCK as libc::c_short);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_fails_fast_until_the_first_releases() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        // Two independent lock tables model two processes (desktop + CLI) sharing a state dir.
        let desktop = PrefixLocks::new(&paths);
        let cli = PrefixLocks::new(&paths);
        let prefix = Uuid::new_v4();

        assert_eq!(desktop.probe(prefix), LockProbe::Unknown);
        let guard = desktop.acquire(prefix, "operation 1").unwrap();
        assert!(cli.is_held(prefix));
        match cli.acquire(prefix, "operation 2") {
            Err(BettertricksError::Busy(holder)) => assert!(holder.contains("operation 1")),
            other => panic!("expected busy, got {other:?}"),
        }
        assert!(cli.acquire(Uuid::new_v4(), "other prefix").is_ok());
        drop(guard);
        assert!(!cli.is_held(prefix));
        assert!(cli.acquire(prefix, "operation 2").is_ok());
    }
}
