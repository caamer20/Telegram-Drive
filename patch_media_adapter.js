const fs = require('fs');
let code = fs.readFileSync('app/src-tauri/src/migration/adapters/media.rs', 'utf8');

code = code.replace(/    \}\n\}\n\}\n\n\/\/ \-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-\-/g, '    }\n}\n\n// ---------------------------------------------------------------------------');

code = code.replace(
    /pub struct FFmpegMediaAdapter \{[\s\S]*?max_threads: usize,\n\}/,
    `pub struct FFmpegMediaAdapter {
    ffprobe_path: PathBuf,
    ffmpeg_path: PathBuf,
    process_runner: Arc<dyn ProcessRunner>,
    cancel_token: Arc<AtomicBool>,
    max_threads: usize,
    app_handle: Option<tauri::AppHandle>,
    hevc_encoder: String,
}`
);

code = code.replace(
    /pub fn new\([\s\S]*?\) -> Self \{[\s\S]*?\}\n\n    \/\/\/ Constructor for tests/m,
    `pub fn new(
        ffprobe_path: PathBuf,
        ffmpeg_path: PathBuf,
        cancel_token: Arc<AtomicBool>,
        max_threads: usize,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        let mut hevc_encoder = "libx265".to_string();
        if cfg!(target_os = "macos") {
            if let Ok(output) = std::process::Command::new(&ffmpeg_path)
                .args(["-encoders"])
                .output() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("hevc_videotoolbox") {
                    hevc_encoder = "hevc_videotoolbox".to_string();
                }
            }
        }

        Self {
            ffprobe_path,
            ffmpeg_path,
            process_runner: Arc::new(RealProcessRunner),
            cancel_token,
            max_threads,
            app_handle,
            hevc_encoder,
        }
    }

    /// Constructor for tests`
);

code = code.replace(
    /pub fn new_with_runner\([\s\S]*?\) -> Self \{[\s\S]*?max_threads,\n        \}/,
    `pub fn new_with_runner(
        ffprobe_path: PathBuf,
        ffmpeg_path: PathBuf,
        process_runner: Arc<dyn ProcessRunner>,
        cancel_token: Arc<AtomicBool>,
        max_threads: usize,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        Self {
            ffprobe_path,
            ffmpeg_path,
            process_runner,
            cancel_token,
            max_threads,
            app_handle,
            hevc_encoder: "libx265".to_string(),
        }`
);

fs.writeFileSync('app/src-tauri/src/migration/adapters/media.rs', code);
