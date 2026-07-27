const fs = require('fs');
let code = fs.readFileSync('app/src-tauri/src/migration/pipeline/runner.rs', 'utf8');

code = code.replace(
    /            \/\/ Enforce flood wait check before uploading[\s\S]*?break;\n\s*\}\n\s*\}/m,
    `            // Enforce flood wait check before uploading
            let wait_until = {
                let conn = db_clone.lock().unwrap();
                let mut stmt = conn.prepare("SELECT flood_wait_until FROM migration_jobs WHERE id = ? LIMIT 1").unwrap();
                stmt.bind((1, item.job_id)).unwrap();
                if let Ok(sqlite::State::Row) = stmt.next() {
                    stmt.read::<i64, _>(0).unwrap_or(0)
                } else {
                    0
                }
            };

            let now = Utc::now().timestamp();
            if wait_until > now {
                let sleep_secs = (wait_until - now) as u64;
                log::info!("Upload: Job {} is under FloodWait, sleeping for {} seconds (non-blocking)", item.job_id, sleep_secs);
                let _ = update_item_pipeline_stage(&db_clone, item.id, PipelineStage::WaitingForQuota);
                let upload_tx_requeue = upload_tx_clone.clone();
                let item_requeue = item.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(tokio::time::Duration::from_secs(sleep_secs)).await;
                    let _ = upload_tx_requeue.send(item_requeue).await;
                });
                continue;
            }`
);

code = code.replace(
    /                \/\/ Quota check — atomic reserve before upload[\s\S]*?match reserved \{[\s\S]*?Ok\(_\) => break,[\s\S]*?Err\(e\) => \{[\s\S]*?\}[\s\S]*?\}[\s\S]*?\}/m,
    `                // Quota check — atomic reserve before upload
                let date_string = Utc::now().format("%Y-%m-%d").to_string();
                let reserved = {
                    let conn = db_clone.lock().unwrap();
                    reserve_quota(
                        &conn,
                        item.id,
                        item.job_id,
                        &date_string,
                        artifact_size,
                        7200, // 2 hour expiry
                    )
                };
                
                match reserved {
                    Ok(_) => {},
                    Err(e) => {
                        log::warn!("Upload: quota reserve failed for item {}: {} — requeuing in 5 mins", item.id, e);
                        let _ = update_item_pipeline_stage(&db_clone, item.id, PipelineStage::WaitingForQuota);
                        let upload_tx_requeue = upload_tx_clone.clone();
                        let item_requeue = item.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
                            let _ = upload_tx_requeue.send(item_requeue).await;
                        });
                        continue;
                    }
                }`
);

fs.writeFileSync('app/src-tauri/src/migration/pipeline/runner.rs', code);
