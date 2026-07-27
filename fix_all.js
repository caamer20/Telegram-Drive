const fs = require('fs');

// Patch runner.rs
let runnerCode = fs.readFileSync('app/src-tauri/src/migration/pipeline/runner.rs', 'utf8');

runnerCode = runnerCode.replace(
    /        let upload_handle = tokio::spawn\(async move \{[\s\S]*?            let _ = runner_clone\.run_uploader\(upload_rx, uploader\)\.await;[\s\S]*?        \}\);/m,
    `        let upload_tx_uploader = upload_tx.clone();
        let upload_handle = tokio::spawn(async move {
            let _ = runner_clone.run_uploader(upload_rx, upload_tx_uploader, uploader).await;
        });`
);

runnerCode = runnerCode.replace(
    /    async fn run_uploader\([\s\S]*?        &self,\n        rx: mpsc::Receiver<PipelineItem>,\n        uploader: Arc<dyn TelegramUploader>,\n    \) -> Result<\(\), String> \{/m,
    `    async fn run_uploader(
        &self,
        rx: mpsc::Receiver<PipelineItem>,
        upload_tx: mpsc::Sender<PipelineItem>,
        uploader: Arc<dyn TelegramUploader>,
    ) -> Result<(), String> {`
);

runnerCode = runnerCode.replace(
    /            let rx_clone = rx\.clone\(\);\n            let db_clone = self\.db\.clone\(\);\n            let uploader_clone = uploader\.clone\(\);/m,
    `            let rx_clone = rx.clone();
            let db_clone = self.db.clone();
            let upload_tx_clone = upload_tx.clone();
            let uploader_clone = uploader.clone();`
);

runnerCode = runnerCode.replace(
    /                            \/\/ Dọn dẹp tệp tin trong workspace\n                            let _ = std::fs::remove_file\(&file_path\);/m,
    `                            // Dọn dẹp tệp tin trong workspace
                            let original_path = workspace.join(format!("{}", item.id));
                            let processed_path = workspace.join(format!("{}.processed.mp4", item.id));
                            let _ = std::fs::remove_file(&original_path);
                            let _ = std::fs::remove_file(&processed_path);`
);

fs.writeFileSync('app/src-tauri/src/migration/pipeline/runner.rs', runnerCode);

// Patch crawler.rs
let crawlerCode = fs.readFileSync('app/src-tauri/src/migration/pipeline/crawler.rs', 'utf8');
crawlerCode = crawlerCode.replace(
    /                            local_artifact_path: None,(\s*)telegram_random_id: None,/g,
    `                            local_artifact_path: None,$1processed_artifact_path: None,$1telegram_random_id: None,`
);
fs.writeFileSync('app/src-tauri/src/migration/pipeline/crawler.rs', crawlerCode);
