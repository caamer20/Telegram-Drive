const fs = require('fs');
let runnerCode = fs.readFileSync('app/src-tauri/src/migration/pipeline/runner.rs', 'utf8');

runnerCode = runnerCode.replace(
    /        \/\/ 3\. Task Processor \(inspect và xử lý video bằng ffmpeg\)\n        let runner_clone = self\.clone\(\);\n        let process_handle = tokio::spawn\(async move \{\n            let _ = runner_clone\n                \.run_processor\(process_rx, upload_tx, inspector, processor\)\n                \.await;\n        \}\);/m,
    `        // 3. Task Processor (inspect và xử lý video bằng ffmpeg)
        let runner_clone = self.clone();
        let upload_tx_processor = upload_tx.clone();
        let process_handle = tokio::spawn(async move {
            let _ = runner_clone
                .run_processor(process_rx, upload_tx_processor, inspector, processor)
                .await;
        });`
);
fs.writeFileSync('app/src-tauri/src/migration/pipeline/runner.rs', runnerCode);
