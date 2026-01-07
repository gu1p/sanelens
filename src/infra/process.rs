use std::env;
use std::io;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) fn command_exists(cmd: &str) -> bool {
    if cmd.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(cmd).is_file();
    }
    if let Ok(path) = env::var("PATH") {
        for entry in env::split_paths(&path) {
            let candidate = entry.join(cmd);
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

pub(crate) fn run_status(cmd: &[String]) -> bool {
    run_output(cmd).map(|output| output.status.success()).unwrap_or(false)
}

pub(crate) fn run_output(cmd: &[String]) -> io::Result<Output> {
    let mut command = Command::new(&cmd[0]);
    command.args(&cmd[1..]).stdout(Stdio::piped()).stderr(Stdio::piped());
    command.output()
}

pub(crate) fn spawn_process_group(cmd: &mut Command) -> io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    cmd.spawn()
}

pub(crate) fn terminate_process(child: &mut Child, timeout: Duration) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        unsafe {
            libc::killpg(pid, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    if wait_child_timeout(child, timeout) {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::killpg(child.id() as i32, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    let _ = wait_child_timeout(child, Duration::from_secs(1));
}

pub(crate) fn wait_child_timeout(child: &mut Child, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if child.try_wait().ok().flatten().is_some() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub(crate) fn pid_alive(pid: i32) -> bool {
    #[cfg(unix)]
    unsafe {
        if libc::kill(pid, 0) == 0 {
            return true;
        }
        let err = io::Error::last_os_error();
        return err.raw_os_error() == Some(libc::EPERM);
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}
