const fs = require('fs');
let code = fs.readFileSync('app/src-tauri/src/migration/adapters/media.rs', 'utf8');

code = code.replace(
    /            let on_progress: Option<Arc<dyn Fn\(&str\) \+ Send \+ Sync>> = if let Some\(app\) = app_handle \{[\s\S]*?Some\(Arc::new\(move \|line: &str\| \{/m,
    `            let on_progress: Option<Arc<dyn Fn(&str) + Send + Sync>> = if let Some(app) = app_handle {
                let last_emit_ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                Some(Arc::new(move |line: &str| {`
);

code = code.replace(
    /                            let payload = serde_json::json\!\(\{[\s\S]*?\}\);[\s\S]*?let _ = app\.emit\("migration:compression-progress", payload\);/m,
    `                            let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                            let last = last_emit_ms.load(std::sync::atomic::Ordering::Relaxed);
                            if now_ms - last >= 250 {
                                last_emit_ms.store(now_ms, std::sync::atomic::Ordering::Relaxed);
                                let payload = serde_json::json!({
                                    "item_id": item_id,
                                    "job_id": job_id,
                                    "progress": percent,
                                    "state": "processing"
                                });
                                let _ = app.emit("migration:compression-progress", payload);
                            }`
);

fs.writeFileSync('app/src-tauri/src/migration/adapters/media.rs', code);
