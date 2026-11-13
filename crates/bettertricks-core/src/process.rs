use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use parking_lot::Mutex;
use std::process::ExitStatus;
use std::time::Duration;

use tokio::process::{Child, Command};

use crate::{BettertricksError, Result};

const TERMINATE_GRACE: Duration = Duration::from_secs(3);
const WINESERVER_KILL_TIMEOUT: Duration = Duration::from_secs(15);

/// Wine processes that must be stopped together on cancellation.
#[derive(Debug, Clone)]
pub(crate) struct WineScope {
    pub wineserver: PathBuf,
    pub prefix: PathBuf,
}

/// Starts `command` in its own process group so cancellation reaches every descendant
/// (installers, msiexec, the wine loader) and not only the direct child.
pub(crate) async fn spawn_supervised(command: &mut Command) -> Result<Child> {
    command.kill_on_drop(true);
    spawn_group(command).await
}

/// Spawns `command` as the leader of a new process group and records the group so it can be
/// stopped when the app or CLI exits ([`stop_all_groups`]). Unlike [`spawn_supervised`] the
/// child outlives a dropped handle (used for tool windows the user keeps open).
pub(crate) async fn spawn_group(command: &mut Command) -> Result<Child> {
    command.process_group(0);
    let child = spawn_retrying(command).await?;
    if let Some(pid) = child.id() {
        let mut groups = GROUPS.lock();
        groups.retain(|group| is_our_group(*group));
        groups.insert(pid as libc::pid_t);
    }
    Ok(child)
}

/// Process groups this process started. Pruned lazily; [`is_our_group`] guards every kill so a
/// recycled pid can never be signalled.
static GROUPS: LazyLock<Mutex<HashSet<libc::pid_t>>> = LazyLock::new(Mutex::default);

/// True while `pid` is still our own child and leads its own group.
fn is_our_group(pid: libc::pid_t) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    // Fields after the parenthesised command name: state ppid pgrp ...
    let fields = stat
        .rsplit_once(')')
        .map(|(_, rest)| rest.split_whitespace().collect::<Vec<_>>())
        .unwrap_or_default();
    // SAFETY: getpid has no preconditions.
    let parent = unsafe { libc::getpid() };
    fields.get(1).and_then(|value| value.parse().ok()) == Some(parent)
        && fields.get(2).and_then(|value| value.parse().ok()) == Some(pid)
}

/// Kills every process group this process started that is still running (installers, Wine
/// tools). Called when the desktop app quits and on a second Ctrl-C in the CLI.
pub fn stop_all_groups() {
    stop_groups(|_| true);
}

fn stop_groups(selected: impl Fn(libc::pid_t) -> bool) {
    let groups = {
        let mut groups = GROUPS.lock();
        let (chosen, kept) = groups
            .drain()
            .partition::<HashSet<_>, _>(|group| selected(*group));
        *groups = kept;
        chosen
    };
    for group in groups {
        if is_our_group(group) {
            // SAFETY: `group` was verified to be a live group led by our own child.
            unsafe { libc::killpg(group, libc::SIGKILL) };
        }
    }
}

/// `Command::spawn`, retried while the executable reports ETXTBSY (see [`retry_text_busy`]).
pub(crate) async fn spawn_retrying(command: &mut Command) -> Result<Child> {
    for _ in 0..50 {
        match command.spawn() {
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            result => return Ok(result?),
        }
    }
    Ok(command.spawn()?)
}

/// Blocking variant for code that already runs on a blocking thread.
///
/// Retries `exec` that fails with ETXTBSY. A script that was just written (a freshly published
/// compatibility host) stays "busy" while any process forked by another thread still holds a
/// copy of its write descriptor between `fork` and `exec`; that window closes on its own.
pub(crate) fn retry_text_busy<T>(
    mut spawn: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    for _ in 0..50 {
        match spawn() {
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return result,
        }
    }
    spawn()
}

/// Polls `child` until it exits or `cancelled()` turns true; on cancellation the whole
/// process group is stopped and the prefix's wineserver is killed before returning.
pub(crate) async fn wait_supervised(
    child: &mut Child,
    cancelled: impl Fn() -> bool,
    scope: Option<&WineScope>,
) -> Result<ExitStatus> {
    loop {
        if cancelled() {
            stop_process_group(child, scope).await;
            return Err(BettertricksError::Cancelled);
        }
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// SIGTERM the group, SIGKILL whatever is left after a grace period, then `wineserver -k`
/// for the prefix and wait for it, so nothing keeps writing once the prefix lock is released.
pub(crate) async fn stop_process_group(child: &mut Child, scope: Option<&WineScope>) {
    if let Some(pid) = child.id() {
        let group = pid as libc::pid_t;
        // SAFETY: plain syscalls on a process group this process created.
        unsafe { libc::killpg(group, libc::SIGTERM) };
        if tokio::time::timeout(TERMINATE_GRACE, child.wait())
            .await
            .is_err()
        {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        // Members that ignored SIGTERM or outlived the leader.
        unsafe { libc::killpg(group, libc::SIGKILL) };
    } else {
        let _ = child.wait().await;
    }
    if let Some(scope) = scope {
        kill_wineserver(scope).await;
    }
}

pub(crate) async fn kill_wineserver(scope: &WineScope) {
    let mut command = Command::new(&scope.wineserver);
    command
        .arg("-k")
        .env("WINEPREFIX", &scope.prefix)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    if let Ok(mut child) = spawn_retrying(&mut command).await {
        let _ = tokio::time::timeout(WINESERVER_KILL_TIMEOUT, child.wait()).await;
    }
}

/// Waits for the prefix's wineserver to flush and exit (`wineserver -w`); returns false when
/// programs are still running in the prefix after `timeout`.
pub(crate) async fn wineserver_idle(wineserver: &Path, prefix: &Path, timeout: Duration) -> bool {
    let mut command = Command::new(wineserver);
    command
        .arg("-w")
        .env("WINEPREFIX", prefix)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let Ok(mut child) = spawn_supervised(&mut command).await else {
        // No wineserver binary means no server can be holding unflushed state.
        return true;
    };
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(_) => true,
        Err(_) => {
            stop_process_group(&mut child, None).await;
            false
        }
    }
}

/// Runs a short probe (`--version` and similar) with a hard timeout.
pub(crate) async fn probe_output(
    command: &mut Command,
    timeout: Duration,
) -> Option<std::process::Output> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = spawn_supervised(command).await.ok()?;
    tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .ok()?
        .ok()
}

/// Test helper: true once `pid` no longer exists or is only a zombie awaiting its reaper.
#[cfg(test)]
pub(crate) async fn process_gone(pid: libc::pid_t) -> bool {
    for _ in 0..100 {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        if stat.is_empty() || stat.contains(") Z ") {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn exit_cleanup_kills_recorded_groups() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("sleep 60");
        let mut child = spawn_group(&mut command).await.unwrap();
        let pid = child.id().unwrap() as libc::pid_t;
        assert!(is_our_group(pid));
        // Only this test's group: other tests in this process have groups of their own.
        stop_groups(|group| group == pid);
        let _ = child.wait().await;
        assert!(process_gone(pid).await);
    }

    #[tokio::test]
    async fn cancellation_kills_grandchildren_and_the_prefix_wineserver() {
        let temp = tempfile::tempdir().unwrap();
        let pid_file = temp.path().join("grandchild.pid");
        let wineserver = temp.path().join("wineserver");
        let killed = temp.path().join("wineserver.args");
        std::fs::write(
            &wineserver,
            format!(
                "#!/bin/sh\necho \"$@ $WINEPREFIX\" >> '{}'\n",
                killed.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wineserver, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!(
            "sleep 60 & echo $! > '{}'; wait",
            pid_file.display()
        ));
        let mut child = spawn_supervised(&mut command).await.unwrap();
        for _ in 0..500 {
            if pid_file.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(pid_file.exists(), "child never started");
        let grandchild: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let scope = WineScope {
            wineserver,
            prefix: temp.path().join("prefix"),
        };

        let result = wait_supervised(&mut child, || true, Some(&scope)).await;

        assert!(matches!(result, Err(BettertricksError::Cancelled)));
        assert!(
            process_gone(grandchild).await,
            "grandchild {grandchild} survived cancellation"
        );
        let calls = std::fs::read_to_string(&killed).unwrap();
        assert!(calls.contains("-k") && calls.contains("prefix"));
    }
}
