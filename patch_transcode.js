const fs = require('fs');
let code = fs.readFileSync('app/src-tauri/src/migration/adapters/media.rs', 'utf8');

code = code.replace(
    /    fn build_transcode_args\([\s\S]*?    \}\n/m,
    `    fn build_transcode_args(
        input_path: &Path,
        output_path: &Path,
        is_10bit: bool,
        encoder: &str,
    ) -> Vec<String> {
        let mut args = vec![
            "-y".to_string(),
            "-i".to_string(),
            input_path.to_string_lossy().to_string(),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            "0:a:0?".to_string(),
            "-sn".to_string(),
            "-dn".to_string(),
            "-vf".to_string(),
            "scale=w='min(1920,iw)':h='min(1080,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2".to_string(),
        ];

        let pix_fmt = if is_10bit { "yuv420p10le" } else { "yuv420p" };
        let profile = if is_10bit { "main10" } else { "main" };

        args.push("-c:v".to_string());
        args.push(encoder.to_string());

        if encoder == "hevc_videotoolbox" {
            args.push("-profile:v".to_string());
            args.push(profile.to_string());
            args.push("-q:v".to_string());
            args.push("60".to_string());
        } else {
            args.push("-preset".to_string());
            args.push("faster".to_string());
            args.push("-crf".to_string());
            args.push("26".to_string());
            args.push("-profile:v".to_string());
            args.push(profile.to_string());
        }

        args.push("-pix_fmt".to_string());
        args.push(pix_fmt.to_string());
        args.push("-c:a".to_string());
        args.push("aac".to_string());
        args.push("-b:a".to_string());
        args.push("128k".to_string());
        args.push("-max_muxing_queue_size".to_string());
        args.push("1024".to_string());
        args.push("-movflags".to_string());
        args.push("+faststart".to_string());
        args.push("-progress".to_string());
        args.push("pipe:1".to_string());
        args.push("-nostats".to_string());
        args.push(output_path.to_string_lossy().to_string());

        args
    }\n`
);

fs.writeFileSync('app/src-tauri/src/migration/adapters/media.rs', code);
