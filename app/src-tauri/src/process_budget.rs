//! Bounded media helpers, including probes; no shell and no inherited stdin.
use std::{
    io::Read,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
#[cfg(not(target_os = "windows"))]
use sysinfo::{Pid, System};

pub(crate) const RESIDENT_LIMIT: u64 = 1024 * 1024 * 1024;
#[cfg(target_os = "linux")]
const ADDRESS_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const CAPTURE_LIMIT: u64 = 8192;
#[derive(Clone, serde::Serialize)]
pub(crate) struct Metrics {
    pub seconds: f64,
    pub sampled_peak_bytes: u64,
    pub os_peak_bytes: Option<u64>,
}
pub(crate) struct Outcome {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub metrics: Metrics,
}
struct Running {
    child: Child,
    running: bool,
    #[cfg(target_os = "windows")]
    job: windows_sys::Win32::Foundation::HANDLE,
}
impl Running {
    #[cfg(target_os = "windows")]
    fn resident_bytes(&self) -> Option<u64> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        // Query the handle returned by CreateProcess, not a reopened PID or a
        // library's inference that an exiting process has ceased to exist.
        unsafe {
            let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            let size = std::mem::size_of_val(&counters) as u32;
            counters.cb = size;
            (GetProcessMemoryInfo(self.child.as_raw_handle(), &mut counters, size) != 0)
                .then_some(counters.WorkingSetSize as u64)
        }
    }
    #[cfg(target_os = "windows")]
    fn exit_started(&self) -> bool {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{Foundation::STILL_ACTIVE, System::Threading::GetExitCodeProcess};
        // Windows assigns the exit code before signaling process completion.
        // A failed query or a still-active child never earns an unmonitored wait.
        unsafe {
            let mut code = STILL_ACTIVE as u32;
            GetExitCodeProcess(self.child.as_raw_handle(), &mut code) != 0
                && code != STILL_ACTIVE as u32
        }
    }
    fn stop(&mut self) -> Option<u64> {
        if !self.running {
            return None;
        }
        self.kill_tree();
        #[cfg(unix)]
        unsafe {
            let mut status = 0;
            let mut usage: libc::rusage = std::mem::zeroed();
            if libc::wait4(self.child.id() as i32, &mut status, 0, &mut usage) < 0 {
                return None;
            }
            self.running = false;
            let peak = usage.ru_maxrss.max(0) as u64;
            #[cfg(target_os = "macos")]
            return Some(peak);
            #[cfg(not(target_os = "macos"))]
            return Some(peak.saturating_mul(1024));
        }
        #[cfg(target_os = "windows")]
        {
            let _ = self.child.wait();
            self.running = false;
            None
        }
    }
    fn kill_tree(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        #[cfg(target_os = "windows")]
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
        }
    }
    fn finish(&mut self) -> Result<Option<(bool, Option<u64>)>, String> {
        #[cfg(unix)]
        unsafe {
            // Observe without reaping, so the process-group ID cannot be reused
            // before descendants are terminated and wait4 collects OS peak RSS.
            let mut info: libc::siginfo_t = std::mem::zeroed();
            if libc::waitid(
                libc::P_PID,
                self.child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            ) != 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            #[cfg(target_os = "linux")]
            let exited = info.si_pid() != 0;
            #[cfg(not(target_os = "linux"))]
            let exited = info.si_pid != 0;
            if !exited {
                return Ok(None);
            }
            self.kill_tree();
            let mut status = 0;
            let mut usage: libc::rusage = std::mem::zeroed();
            if libc::wait4(self.child.id() as i32, &mut status, 0, &mut usage) < 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            self.running = false;
            let peak = usage.ru_maxrss.max(0) as u64;
            #[cfg(target_os = "macos")]
            let bytes = peak;
            #[cfg(not(target_os = "macos"))]
            let bytes = peak.saturating_mul(1024);
            Ok(Some((
                libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
                Some(bytes),
            )))
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                self.running = false;
                self.kill_tree();
                return Ok(Some((status.success(), None)));
            }
            Ok(None)
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        if self.running {
            self.stop();
        }
        #[cfg(target_os = "windows")]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}
#[cfg(target_os = "windows")]
fn attach_job(child: &mut Child) -> Result<windows_sys::Win32::Foundation::HANDLE, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::*,
        System::{Diagnostics::ToolHelp::*, JobObjects::*, Threading::*},
    };
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("HEIC_PROCESS_LIMIT".into());
        }
        let setup = (|| {
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_JOB_MEMORY
                | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            limits.JobMemoryLimit = RESIDENT_LIMIT as usize;
            limits.ProcessMemoryLimit = RESIDENT_LIMIT as usize;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(job, child.as_raw_handle()) == 0
            {
                return Err("HEIC_PROCESS_LIMIT".into());
            }
            // CREATE_SUSPENDED prevents executable code from running before
            // job attachment. The primary thread is the only resumable thread.
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err("HEIC_PROCESS_LIMIT".into());
            }
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            let mut thread_id = None;
            let mut more = Thread32First(snapshot, &mut entry);
            while more != 0 {
                if entry.th32OwnerProcessID == child.id() {
                    if thread_id.is_some() {
                        CloseHandle(snapshot);
                        return Err("HEIC_PROCESS_LIMIT".into());
                    }
                    thread_id = Some(entry.th32ThreadID);
                }
                more = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            let thread = OpenThread(
                THREAD_SUSPEND_RESUME,
                0,
                thread_id.ok_or("HEIC_PROCESS_LIMIT")?,
            );
            if thread.is_null() {
                return Err("HEIC_PROCESS_LIMIT".into());
            }
            let previous = ResumeThread(thread);
            CloseHandle(thread);
            if previous != 1 {
                return Err("HEIC_PROCESS_LIMIT".into());
            }
            Ok(())
        })();
        if let Err(error) = setup {
            // Covers assignment, discovery and resume errors after suspended creation.
            let _ = child.kill();
            let _ = child.wait();
            CloseHandle(job);
            return Err(error);
        }
        Ok(job)
    }
}

pub(crate) fn run(
    command: Command,
    deadline: Instant,
    capture: bool,
    check: impl Fn() -> Result<(), String>,
    output: Option<(&std::path::Path, u64)>,
) -> Result<Outcome, String> {
    run_inner(
        command,
        deadline,
        capture,
        check,
        output,
        #[cfg(feature = "native-e2e")]
        None,
    )
}
#[cfg(feature = "native-e2e")]
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FixtureMonitorLoss {
    BeforeCompletion,
    AfterCompletion,
}
#[cfg(feature = "native-e2e")]
#[derive(Clone)]
pub(crate) struct FixtureClock {
    pub ready: std::path::PathBuf,
    pub delay_ms: u64,
    pub duration: Duration,
    pub monitor_loss: Option<FixtureMonitorLoss>,
    pub exit_delay_ms: u64,
    pub exit_failure: bool,
    pub sample_gate: Option<(std::path::PathBuf, std::path::PathBuf)>,
}
#[cfg(feature = "native-e2e")]
pub(crate) fn run_fixture(
    mut command: Command,
    capture: bool,
    check: impl Fn() -> Result<(), String>,
    output: Option<(&std::path::Path, u64)>,
    clock: &FixtureClock,
) -> Result<Outcome, String> {
    command
        .env("TD_E2E_CONVERSION_READY", &clock.ready)
        .env("TD_E2E_READY_DELAY_MS", clock.delay_ms.to_string())
        .env("TD_E2E_COMPLETE", clock.ready.with_extension("complete"))
        .env("TD_E2E_EXIT_DELAY_MS", clock.exit_delay_ms.to_string())
        .env(
            "TD_E2E_EXIT_FAILURE",
            if clock.exit_failure { "1" } else { "0" },
        );
    if clock.monitor_loss == Some(FixtureMonitorLoss::AfterCompletion) {
        command.env("TD_E2E_EXIT_ACK", clock.ready.with_extension("ack"));
    }
    run_inner(
        command,
        Instant::now() + Duration::from_secs(120),
        capture,
        check,
        output,
        Some(clock),
    )
}
fn run_inner(
    mut command: Command,
    deadline: Instant,
    capture: bool,
    check: impl Fn() -> Result<(), String>,
    output: Option<(&std::path::Path, u64)>,
    #[cfg(feature = "native-e2e")] fixture: Option<&FixtureClock>,
) -> Result<Outcome, String> {
    check()?;
    #[cfg(feature = "native-e2e")]
    let command_name = command.get_program().to_string_lossy().into_owned();
    #[cfg(feature = "native-e2e")]
    let version_probe = command.get_args().any(|arg| arg == "-version");
    // Only an explicitly configured native fixture can declare completed work.
    #[cfg(feature = "native-e2e")]
    let completion = command.get_envs().find_map(|(key, value)| {
        (key == "TD_E2E_COMPLETE")
            .then(|| value.map(std::path::PathBuf::from))
            .flatten()
    });
    command.stdin(Stdio::null()).stderr(Stdio::null());
    command.stdout(if capture {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    crate::process_util::hide_console_blocking(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                #[cfg(target_os = "linux")]
                {
                    let limit = libc::rlimit {
                        rlim_cur: ADDRESS_LIMIT as libc::rlim_t,
                        rlim_max: ADDRESS_LIMIT as libc::rlim_t,
                    };
                    if libc::setrlimit(libc::RLIMIT_AS, &limit) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000 | 0x0000_0004); // NO_WINDOW | SUSPENDED
    }
    let started = Instant::now();
    let child = command.spawn().map_err(|_| "HEIC_DECODER_UNAVAILABLE")?;
    #[cfg(target_os = "windows")]
    let mut child = child;
    #[cfg(target_os = "windows")]
    let job = attach_job(&mut child)?;
    let mut running = Running {
        child,
        running: true,
        #[cfg(target_os = "windows")]
        job,
    };
    // Install cleanup before allocating the probe reader thread.
    let reader = running
        .child
        .stdout
        .take()
        .map(|stdout| {
            std::thread::Builder::new()
                .name("heic-probe".into())
                .spawn(move || {
                    let mut bytes = Vec::new();
                    stdout
                        .take(CAPTURE_LIMIT + 1)
                        .read_to_end(&mut bytes)
                        .map(|_| bytes)
                })
                .map_err(|_| "HEIC_PROBE_FAILED".to_string())
        })
        .transpose()?;
    #[cfg(feature = "native-e2e")]
    let mut readiness_seconds = None;
    #[cfg(feature = "native-e2e")]
    let mut clock_started = None;
    #[cfg(feature = "native-e2e")]
    let mut sampler_seconds = 0.0;
    #[cfg(feature = "native-e2e")]
    let mut sample_gate_used = false;
    #[cfg(feature = "native-e2e")]
    let mut completed_exit_wait = false;
    #[cfg(feature = "native-e2e")]
    let spawn_seconds = started.elapsed().as_secs_f64();
    #[cfg(not(target_os = "windows"))]
    let mut system = System::new();
    #[cfg(not(target_os = "windows"))]
    let pid = Pid::from_u32(running.child.id());
    let mut peak = 0;
    let observed = (|| -> Result<(bool, Option<u64>), String> {
        loop {
            check()?;
            #[cfg(feature = "native-e2e")]
            let current_deadline = if let Some(clock) = fixture {
                if clock_started.is_none()
                    && std::fs::read_to_string(&clock.ready)
                        .is_ok_and(|value| value == running.child.id().to_string())
                {
                    let ready = Instant::now();
                    clock_started = Some(ready);
                    readiness_seconds = Some(started.elapsed().as_secs_f64());
                }
                clock_started
                    .map(|ready| ready + clock.duration)
                    .unwrap_or(deadline)
            } else {
                deadline
            };
            #[cfg(not(feature = "native-e2e"))]
            let current_deadline = deadline;
            if Instant::now() >= current_deadline {
                return Err("HEIC_DECODE_DEADLINE".into());
            }
            if let Some(result) = running.finish()? {
                return Ok(result);
            }
            #[cfg(feature = "native-e2e")]
            if !sample_gate_used && clock_started.is_some() {
                if let Some((started, release)) =
                    fixture.and_then(|clock| clock.sample_gate.as_ref())
                {
                    sample_gate_used = true;
                    // Hold a controlled RSS read while the real request is cancelled.
                    // A missing release remains bounded and reaches normal containment.
                    if std::fs::write(started, running.child.id().to_string()).is_ok() {
                        let until = Instant::now() + Duration::from_secs(120);
                        while !release.is_file() && Instant::now() < until {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
            }
            #[cfg(feature = "native-e2e")]
            let sampled = Instant::now();
            #[cfg(not(target_os = "windows"))]
            let refreshed = system.refresh_process(pid);
            #[cfg(target_os = "windows")]
            let memory = running.resident_bytes();
            #[cfg(target_os = "windows")]
            let refreshed = memory.is_some();
            #[cfg(feature = "native-e2e")]
            let completed = completion.as_ref().is_some_and(|path| {
                std::fs::read_to_string(path)
                    .is_ok_and(|value| value == running.child.id().to_string())
            });
            #[cfg(feature = "native-e2e")]
            let refreshed = refreshed
                && !fixture.is_some_and(|clock| match clock.monitor_loss {
                    Some(FixtureMonitorLoss::BeforeCompletion) => clock_started.is_some(),
                    Some(FixtureMonitorLoss::AfterCompletion) => completed,
                    None => false,
                });
            #[cfg(feature = "native-e2e")]
            {
                sampler_seconds += sampled.elapsed().as_secs_f64();
            }
            if refreshed {
                #[cfg(not(target_os = "windows"))]
                if let Some(process) = system.process(pid) {
                    peak = peak.max(process.memory());
                }
                #[cfg(target_os = "windows")]
                {
                    peak = peak.max(memory.unwrap_or(0));
                }
            } else if let Some(result) = running.finish()? {
                return Ok(result);
            } else {
                // Cancellation can arrive during the RSS read. Preserve that
                // reason before classifying a still-live child as unmonitored.
                check()?;
                #[cfg(any(target_os = "windows", feature = "native-e2e"))]
                {
                    #[cfg(all(target_os = "windows", feature = "native-e2e"))]
                    let finishing = running.exit_started() || completed;
                    #[cfg(all(target_os = "windows", not(feature = "native-e2e")))]
                    let finishing = running.exit_started();
                    #[cfg(not(target_os = "windows"))]
                    let finishing = completed;
                    if finishing {
                        // The process has begun exiting (or explicitly finished work
                        // in a controlled fixture). Require its actual exit status;
                        // the Windows Job still enforces memory and process limits.
                        #[cfg(feature = "native-e2e")]
                        {
                            completed_exit_wait = true;
                            if let Some(clock) = fixture {
                                if clock.monitor_loss == Some(FixtureMonitorLoss::AfterCompletion) {
                                    std::fs::write(
                                        clock.ready.with_extension("ack"),
                                        running.child.id().to_string(),
                                    )
                                    .map_err(|_| "HEIC_MEMORY_MONITOR_UNAVAILABLE")?;
                                }
                            }
                        }
                        let exit_deadline =
                            current_deadline.min(Instant::now() + Duration::from_secs(2));
                        loop {
                            check()?;
                            if Instant::now() >= exit_deadline {
                                return Err("HEIC_MEMORY_MONITOR_UNAVAILABLE".into());
                            }
                            if let Some((path, cap)) = output {
                                if std::fs::symlink_metadata(path).is_ok_and(|meta| {
                                    !meta.file_type().is_file() || meta.len() > cap
                                }) {
                                    return Err("HEIC_OUTPUT_LIMIT".into());
                                }
                            }
                            if let Some(result) = running.finish()? {
                                if result.1.is_some_and(|bytes| bytes > RESIDENT_LIMIT) {
                                    return Err("HEIC_MEMORY_LIMIT".into());
                                }
                                return Ok(result);
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
                return Err("HEIC_MEMORY_MONITOR_UNAVAILABLE".into());
            }
            if peak > RESIDENT_LIMIT {
                return Err("HEIC_MEMORY_LIMIT".into());
            }
            if let Some((path, cap)) = output {
                if std::fs::symlink_metadata(path)
                    .is_ok_and(|meta| !meta.file_type().is_file() || meta.len() > cap)
                {
                    return Err("HEIC_OUTPUT_LIMIT".into());
                }
            }
            if let Some(result) = running.finish()? {
                return Ok(result);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    let (success, os_peak) = match observed {
        Ok(result) => result,
        Err(error) => {
            let os_peak = running.stop();
            #[cfg(feature = "native-e2e")]
            observe(
                &command_name,
                version_probe,
                &error,
                running.child.id(),
                (
                    spawn_seconds,
                    readiness_seconds,
                    clock_started.map(|ready| ready.elapsed().as_secs_f64()),
                    sampler_seconds,
                    completed_exit_wait,
                ),
                Metrics {
                    seconds: started.elapsed().as_secs_f64(),
                    sampled_peak_bytes: peak,
                    os_peak_bytes: os_peak,
                },
            );
            #[cfg(not(feature = "native-e2e"))]
            let _ = os_peak;
            return Err(error);
        }
    };
    let stdout = reader
        .map(|reader| {
            reader
                .join()
                .map_err(|_| "HEIC_PROBE_FAILED")?
                .map_err(|_| "HEIC_PROBE_FAILED")
        })
        .transpose()?
        .unwrap_or_default();
    if stdout.len() as u64 > CAPTURE_LIMIT {
        return Err("HEIC_PROBE_LIMIT".into());
    }
    check()?;
    #[cfg(feature = "native-e2e")]
    observe(
        &command_name,
        version_probe,
        if success { "success" } else { "decoder_failed" },
        running.child.id(),
        (
            spawn_seconds,
            readiness_seconds,
            clock_started.map(|ready| ready.elapsed().as_secs_f64()),
            sampler_seconds,
            completed_exit_wait,
        ),
        Metrics {
            seconds: started.elapsed().as_secs_f64(),
            sampled_peak_bytes: peak,
            os_peak_bytes: os_peak,
        },
    );
    Ok(Outcome {
        success,
        stdout,
        metrics: Metrics {
            seconds: started.elapsed().as_secs_f64(),
            sampled_peak_bytes: peak,
            os_peak_bytes: os_peak,
        },
    })
}

#[cfg(feature = "native-e2e")]
#[derive(Clone, serde::Serialize)]
pub(crate) struct Observation {
    executable: String,
    version_probe: bool,
    outcome: String,
    pid: u32,
    metrics: Metrics,
    spawn_seconds: f64,
    readiness_seconds: Option<f64>,
    clock_seconds: Option<f64>,
    sampler_seconds: f64,
    completed_exit_wait: bool,
}
#[cfg(feature = "native-e2e")]
static OBSERVATIONS: std::sync::LazyLock<std::sync::Mutex<Vec<Observation>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));
#[cfg(feature = "native-e2e")]
fn observe(
    executable: &str,
    version_probe: bool,
    outcome: &str,
    pid: u32,
    timing: (f64, Option<f64>, Option<f64>, f64, bool),
    metrics: Metrics,
) {
    OBSERVATIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Observation {
            executable: executable.into(),
            version_probe,
            outcome: outcome.into(),
            pid,
            metrics,
            spawn_seconds: timing.0,
            readiness_seconds: timing.1,
            clock_seconds: timing.2,
            sampler_seconds: timing.3,
            completed_exit_wait: timing.4,
        });
}
#[cfg(feature = "native-e2e")]
pub(crate) fn observations() -> Vec<Observation> {
    OBSERVATIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}
