const fs = require('fs');
let code = fs.readFileSync('app/src-tauri/src/migration/adapters/media.rs', 'utf8');

code = code.replace(
    /impl VideoProcessor for FFmpegMediaAdapter \{[\s\S]*?            if output.exit_code != 0 \{/m,
    `impl VideoProcessor for FFmpegMediaAdapter {
    fn process_video(
        &self,
        input_path: &Path,
        output_path: &Path,
        decision: &str,
        item_id: i64,
        job_id: i64,
        duration: f64,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
        let ffmpeg = self.ffmpeg_path.to_string_lossy().to_string();
        let args = match decision {
            "remux_copy" | "canonical_passthrough_main8" | "canonical_passthrough_main10" => Self::build_remux_args(input_path, output_path),
            "canonical_transcode_main8" | "transcode" => Self::build_transcode_args(input_path, output_path, false, &self.hevc_encoder),
            "canonical_transcode_main10" => Self::build_transcode_args(input_path, output_path, true, &self.hevc_encoder),
            other => {
                let msg = format!("Processor: unsupported decision: {}", other);
                return Box::pin(async move { Err(msg) });
            }
        };
        let runner = self.process_runner.clone();
        let cancel = self.cancel_token.clone();
        let output_path_owned = output_path.to_path_buf();
        let app_handle = self.app_handle.clone();
        let duration_us = duration * 1_000_000.0;

        Box::pin(async move {
            if cancel.load(Ordering::Relaxed) {
                return Err("Processor: cancelled".to_string());
            }

            let on_progress: Option<Arc<dyn Fn(&str) + Send + Sync>> = if let Some(app) = app_handle {
                Some(Arc::new(move |line: &str| {
                    if let Some(time_us_str) = line.strip_prefix("out_time_us=") {
                        if let Ok(time_us) = time_us_str.trim().parse::<f64>() {
                            let percent = if duration_us > 0.0 {
                                (time_us / duration_us * 100.0).clamp(0.0, 100.0)
                            } else {
                                0.0
                            };
                            use tauri::Manager;
                            let payload = serde_json::json!({
                                "item_id": item_id,
                                "job_id": job_id,
                                "progress": percent,
                                "state": "processing"
                            });
                            let _ = app.emit_all("migration:compression-progress", payload);
                        }
                    }
                }))
            } else {
                None
            };

            let output = runner
                .run_command(&ffmpeg, &args, on_progress)
                .await
                .map_err(|e| format!("Processor: ffmpeg spawn error: {}", e))?;

            if output.exit_code != 0 {`
);

fs.writeFileSync('app/src-tauri/src/migration/adapters/media.rs', code);
