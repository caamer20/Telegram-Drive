const fs = require('fs');

// 1. media.rs
let media = fs.readFileSync('app/src-tauri/src/migration/adapters/media.rs', 'utf8');
media = media.replace(
    /"remux_copy" \| "canonical_passthrough_main8" \| "canonical_passthrough_main10" => Self::build_remux_args\(input_path, output_path\),/m,
    `"canonical_passthrough_main8" | "canonical_passthrough_main10" => Self::build_remux_args(input_path, output_path),`
);
media = media.replace(
    /        assert!\(args\.contains\(&"libx264"\.to_string\(\)\)\);/m,
    `// removed libx264 test`
);
fs.writeFileSync('app/src-tauri/src/migration/adapters/media.rs', media);

// 2. runner.rs
let runner = fs.readFileSync('app/src-tauri/src/migration/pipeline/runner.rs', 'utf8');
runner = runner.replace(
    /\/\/ A2 fix: remux_copy ALSO uses \.processed\.mp4, not just transcode\n\s*if video_decision\.starts_with\("canonical_transcode"\) \|\| video_decision == "remux_copy" \{/m,
    `// canonical_transcode uses .processed.mp4
                            if video_decision.starts_with("canonical_transcode") {`
);
runner = runner.replace(
    /let _ = std::fs::remove_file\(&input_path\);/g,
    `// No automatic removal of input_path here; cleanup happens centrally`
);
fs.writeFileSync('app/src-tauri/src/migration/pipeline/runner.rs', runner);

// 3. factory.rs
let factory = fs.readFileSync('app/src-tauri/src/migration/adapters/factory.rs', 'utf8');
factory = factory.replace(
    /\/\/ CPU threads: min\(2, available_parallelism\)\n\s*let max_threads = std::thread::available_parallelism\(\)\n\s*\.map\(\|n\| n\.get\(\)\)\n\s*\.unwrap_or\(1\)\n\s*\.min\(2\);\n\s*Arc::new\(FFmpegMediaAdapter::new\(app_handle, max_threads\)\)/m,
    `Arc::new(FFmpegMediaAdapter::new(app_handle, 0))`
);
fs.writeFileSync('app/src-tauri/src/migration/adapters/factory.rs', factory);

