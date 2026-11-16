//! Controlled JPEG/process fixture, not a HEIF decoder. No app library or async runtime.
use std::{path::PathBuf, time::Duration};
fn main() {
    let result = run();
    if let Err(error) = complete() {
        eprintln!("Controlled media helper completion: {error}");
        std::process::exit(1);
    }
    if let Err(error) = result {
        eprintln!("Controlled HEIC helper: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::current_exe()?
        .parent()
        .ok_or("Missing root")?
        .canonicalize()?;
    if std::fs::read_to_string(root.join(".native-e2e-fixture"))?
        != "telegram-drive-synthetic-e2e\n"
    {
        return Err("Not a fixture".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if root.join("ffmpeg-mode").is_file() {
        if args.get(1).map(String::as_str) == Some("-version") {
            println!("ffmpeg version 8.1.2 controlled native E2E helper");
            return Ok(());
        }
        return video(&root, &args);
    }
    let mode = std::fs::read_to_string(root.join("heic-mode")).unwrap_or_else(|_| "success".into());
    if args.get(1).map(String::as_str) == Some("-version") {
        let version = std::fs::read_to_string(root.join("heic-version")).unwrap_or_else(|_| {
            if mode.trim() == "old" {
                "8.0.1"
            } else {
                "8.1.2"
            }
            .into()
        });
        println!("ffmpeg version {}", version.trim());
        return Ok(());
    }
    std::fs::write(root.join("heic-pid"), std::process::id().to_string())?;
    let delay = std::env::var("TD_E2E_READY_DELAY_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    std::thread::sleep(Duration::from_millis(delay));
    if let Some(ready) = std::env::var_os("TD_E2E_CONVERSION_READY") {
        let ready = PathBuf::from(ready);
        if parent(&ready)? != root {
            return Err("Ready outside fixture".into());
        }
        std::fs::write(ready, std::process::id().to_string())?;
    }
    std::fs::write(root.join("heic-ready"), std::process::id().to_string())?;
    match mode.trim() {
        "gate" => {
            let until = std::time::Instant::now() + Duration::from_secs(120);
            while !root.join("heic-release").is_file() {
                if std::time::Instant::now() >= until {
                    return Err("Fixture gate deadline".into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        "fail" => return Err("Controlled failure".into()),
        "wait" => loop {
            std::thread::sleep(Duration::from_millis(20));
        },
        "memory" => {
            const SIZE: usize = 1032 * 1024 * 1024;
            let layout = std::alloc::Layout::from_size_align(SIZE, 4096)?;
            let memory = unsafe { std::alloc::alloc_zeroed(layout) };
            if memory.is_null() {
                std::fs::write(root.join("heic-memory-denied"), SIZE.to_string())?;
                return Err("Controlled allocation refused by process limit".into());
            }
            // Touch every page immediately; no debug Vec resize or artificial ramp sleeps.
            for offset in (0..SIZE).step_by(4096) {
                unsafe {
                    memory.add(offset).write_volatile(1);
                }
            }
            std::hint::black_box(memory);
            loop {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        _ => {}
    }
    let output = PathBuf::from(args.last().ok_or("Missing output")?);
    if !parent(&output)?.starts_with(&root) {
        return Err("Output outside fixture".into());
    }
    if mode.trim() == "oversize" {
        std::fs::File::create(output)?.set_len(9 * 1024 * 1024)?;
        return Ok(());
    }
    let dimensions = args
        .windows(2)
        .find(|pair| pair[0] == "-s")
        .map(|pair| pair[1].as_str())
        .ok_or("Missing output dimensions")?;
    let frame = if mode.trim() == "tile" {
        root.join("heic-payload-tile.jpg")
    } else {
        root.join(format!("heic-payload-{dimensions}.jpg"))
    };
    std::fs::copy(frame, output)?;
    Ok(())
}

// Completion is a fixture protocol, never proof of success. Memory and active wait
// modes cannot declare it; the parent must still reap the actual exit status.
fn complete() -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = std::env::var_os("TD_E2E_COMPLETE") else {
        return Ok(());
    };
    let path = PathBuf::from(path);
    let root = std::env::current_exe()?
        .parent()
        .ok_or("Missing root")?
        .canonicalize()?;
    if parent(&path)? != root
        || std::fs::read_to_string(root.join(".native-e2e-fixture"))?
            != "telegram-drive-synthetic-e2e\n"
    {
        return Err("Completion outside fixture".into());
    }
    if std::env::args().nth(1).as_deref() != Some("-version")
        && std::fs::read_to_string(root.join("heic-mode")).is_ok_and(|mode| {
            mode.trim() == "memory"
                || mode.trim() == "wait"
                || (mode.trim() == "gate" && !root.join("heic-release").is_file())
        })
    {
        return Ok(());
    }
    std::fs::write(path, std::process::id().to_string())?;
    if let Some(ack) = std::env::var_os("TD_E2E_EXIT_ACK") {
        let ack = PathBuf::from(ack);
        if parent(&ack)? != root {
            return Err("Acknowledgement outside fixture".into());
        }
        let until = std::time::Instant::now() + Duration::from_secs(120);
        while !std::fs::read_to_string(&ack)
            .is_ok_and(|value| value == std::process::id().to_string())
        {
            if std::time::Instant::now() >= until {
                return Err("Completion acknowledgement deadline".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let delay = std::env::var("TD_E2E_EXIT_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .min(1000);
    std::thread::sleep(Duration::from_millis(delay));
    if std::env::var("TD_E2E_EXIT_FAILURE").is_ok_and(|value| value == "1") {
        return Err("Controlled nonzero exit after completed work".into());
    }
    Ok(())
}
fn parent(path: &std::path::Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(path
        .parent()
        .ok_or("Missing output parent")?
        .canonicalize()?)
}

fn video(root: &std::path::Path, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(args.last().ok_or("Missing output")?);
    if !parent(&output)?.starts_with(root) {
        return Err("Output outside fixture".into());
    }
    let mode = std::fs::read_to_string(root.join("ffmpeg-mode"))?;
    std::fs::write(root.join("ffmpeg-pid"), std::process::id().to_string())?;
    let delay = std::env::var("TD_E2E_VIDEO_READY_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .min(30_000);
    std::thread::sleep(Duration::from_millis(delay));
    if let Some(path) = std::env::var_os("TD_E2E_VIDEO_READY") {
        let ready = PathBuf::from(path);
        if parent(&ready)? != root {
            return Err("Video readiness outside fixture".into());
        }
        std::fs::write(ready, std::process::id().to_string())?;
    }
    match mode.trim() {
        "wait" => loop {
            std::thread::sleep(Duration::from_millis(50));
        },
        "frame-wait" => {
            std::fs::copy(root.join("ffmpeg-frame.jpg"), output)?;
            std::fs::write(root.join("ffmpeg-frame-ready"), b"ready")?;
            loop {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        "fail" => return Err("Controlled codec failure".into()),
        _ => {
            std::fs::copy(root.join("ffmpeg-frame.jpg"), output)?;
        }
    }
    Ok(())
}
