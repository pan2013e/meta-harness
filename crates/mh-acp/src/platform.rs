use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::AcpError;

/// Resolves an executable command across Windows (`.exe`, `.cmd`, `.bat`, `.ps1`),
/// macOS, and Linux via `which` before spawning.
pub fn resolve_command(cmd: &str, args: &[String]) -> Result<(PathBuf, Vec<String>), AcpError> {
    let resolved = if Path::new(cmd).components().count() > 1 {
        let p = PathBuf::from(cmd);
        if p.exists() {
            p
        } else {
            return Err(AcpError::CommandNotFound(cmd.to_string()));
        }
    } else {
        which::which(cmd).map_err(|_| AcpError::CommandNotFound(cmd.to_string()))?
    };

    #[cfg(windows)]
    {
        if let Some(ext) = resolved.extension().and_then(|e| e.to_str())
            && ext.eq_ignore_ascii_case("ps1")
        {
            let mut wrapped = vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                dunce::simplified(&resolved).to_string_lossy().into_owned(),
            ];
            wrapped.extend(args.iter().cloned());
            let ps =
                which::which("powershell.exe").unwrap_or_else(|_| PathBuf::from("powershell.exe"));
            return Ok((ps, wrapped));
        }
    }

    Ok((dunce::simplified(&resolved).to_path_buf(), args.to_vec()))
}

/// Checks whether a command (or any of its configured fallbacks) can be resolved on `PATH`.
pub fn probe_executable(cmd: &str) -> Option<PathBuf> {
    if Path::new(cmd).components().count() > 1 {
        let p = PathBuf::from(cmd);
        p.exists().then_some(p)
    } else {
        which::which(cmd).ok()
    }
}

/// Returns the user's preferred interactive shell `(program, args)` on Windows, macOS, and Linux.
pub fn default_shell() -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        if let Ok(pwsh) = which::which("pwsh.exe") {
            return (pwsh.to_string_lossy().into_owned(), vec!["-NoLogo".into()]);
        }
        if let Ok(ps) = which::which("powershell.exe") {
            return (ps.to_string_lossy().into_owned(), vec!["-NoLogo".into()]);
        }
        ("cmd.exe".to_string(), Vec::new())
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        (shell, Vec::new())
    }
}

/// Builds a shell invocation `(program, args)` for running a single command string in `terminal/*`.
pub fn shell_command_invocation(raw_command: &str) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "cmd.exe".to_string(),
            vec!["/D".into(), "/S".into(), "/C".into(), raw_command.into()],
        )
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        (shell, vec!["-c".into(), raw_command.into()])
    }
}

/// Opens a URL in the user's default browser on Windows, macOS, or Linux.
pub fn open_url_in_browser(url: &str) -> Result<(), AcpError> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "start", "", url])
            .spawn()?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
        return Ok(());
    }
}

/// Opens a local terminal window rooted in `dir` across Windows, macOS, and Linux.
pub fn open_terminal_in_dir(dir: &Path) -> Result<(), AcpError> {
    let dir_clean = dunce::simplified(dir);
    #[cfg(windows)]
    {
        if which::which("wt.exe").is_ok() {
            std::process::Command::new("wt.exe")
                .args(["-d"])
                .arg(dir_clean)
                .spawn()?;
        } else {
            std::process::Command::new("cmd.exe")
                .args(["/C", "start", "powershell.exe", "-NoExit"])
                .current_dir(dir_clean)
                .spawn()?;
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-a", "Terminal"])
            .arg(dir_clean)
            .spawn()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for (term, args) in [
            ("x-terminal-emulator", vec![]),
            ("gnome-terminal", vec!["--working-directory"]),
            ("konsole", vec!["--workdir"]),
            ("xterm", vec![]),
        ] {
            if which::which(term).is_ok() {
                let mut c = std::process::Command::new(term);
                if !args.is_empty() {
                    c.args(&args).arg(dir_clean);
                } else {
                    c.current_dir(dir_clean);
                }
                c.spawn()?;
                return Ok(());
            }
        }
        Err(AcpError::CommandNotFound(
            "no terminal emulator found on PATH".into(),
        ))
    }
}

/// Configures OS-specific process-group isolation on a `Command` prior to spawning.
pub fn prepare_isolated_command(cmd: &mut Command) {
    #[cfg(unix)]
    {
        cmd.process_group(0);
        #[cfg(target_os = "linux")]
        {
            // SAFETY: prctl(PR_SET_PDEATHSIG, SIGTERM) is async-signal-safe in pre_exec
            // and guarantees child termination if the parent Rust process dies.
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = cmd;
    }
}

/// Cross-platform child process tree lifetime guard.
/// - Windows: binds child process into a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
/// - macOS & Linux: tracks the dedicated process group ID and sends `SIGTERM` -> `SIGKILL` to `killpg`.
pub struct ProcessTreeGuard {
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(unix)]
    pgid: Option<i32>,
}

// SAFETY: On Windows, the owned Win32 Job Object handle is synchronized by external owner ownership
// and only closed on Drop or terminated via TerminateJobObject.
#[cfg(windows)]
unsafe impl Send for ProcessTreeGuard {}
#[cfg(windows)]
unsafe impl Sync for ProcessTreeGuard {}

impl ProcessTreeGuard {
    pub fn attach(child: &tokio::process::Child) -> Self {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
            use windows_sys::Win32::System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            };

            let mut job: HANDLE = std::ptr::null_mut();
            // SAFETY: Standard Win32 Job Object creation and handle assignment for the spawned child.
            unsafe {
                let created = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if !created.is_null() {
                    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                    let ok = SetInformationJobObject(
                        created,
                        JobObjectExtendedLimitInformation,
                        (&raw const info).cast(),
                        std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                    );
                    if ok != 0
                        && let Some(raw_proc) = child.raw_handle()
                    {
                        let proc_handle: HANDLE = raw_proc.cast();
                        if AssignProcessToJobObject(created, proc_handle) != 0 {
                            job = created;
                        }
                    }
                    if job.is_null() {
                        CloseHandle(created);
                    }
                }
            }
            Self { job }
        }

        #[cfg(unix)]
        {
            let pgid = child.id().map(|id| id as i32);
            Self { pgid }
        }
    }

    pub fn kill_tree(&mut self) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::JobObjects::TerminateJobObject;
            if !self.job.is_null() {
                // SAFETY: `self.job` is a valid owned Job Object handle.
                unsafe {
                    TerminateJobObject(self.job, 1);
                }
            }
        }
        #[cfg(unix)]
        {
            if let Some(pgid) = self.pgid.take() {
                // SAFETY: `pgid` was created via `process_group(0)` matching the child PID.
                unsafe {
                    libc::killpg(pgid, libc::SIGTERM);
                }
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(120));
                    // SAFETY: Escalate to SIGKILL if any grandchild ignored SIGTERM.
                    unsafe {
                        libc::killpg(pgid, libc::SIGKILL);
                    }
                });
            }
        }
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        self.kill_tree();
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::CloseHandle;
            if !self.job.is_null() {
                // SAFETY: Closing our unique Job Object handle triggers KILL_ON_JOB_CLOSE too.
                unsafe {
                    CloseHandle(self.job);
                }
                self.job = std::ptr::null_mut();
            }
        }
    }
}
