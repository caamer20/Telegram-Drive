const fs = require('fs');
let code = fs.readFileSync('app/src-tauri/src/migration/pipeline/runner.rs', 'utf8');

code = code.replace(
    /                let date_string = Utc::now\(\)\.format\("%Y-%m-%d"\)\.to_string\(\);[\s\S]*?            let _ = update_item_pipeline_stage\(&db_clone, item\.id, PipelineStage::Uploading\);/m,
    `                let date_string = Utc::now().format("%Y-%m-%d").to_string();
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
                }
                
                if cancel.is_cancelled() || cancel.is_stopped() {
                    return; // Exit worker
                }

                let _ = update_item_pipeline_stage(&db_clone, item.id, PipelineStage::Uploading);`
);

code = code.replace(
    /                let \(file_path, artifact_size\) = \{[\s\S]*?\(path, size\)[\s\S]*?\};/m,
    `                let (file_path, artifact_size) = {
                    let path = if let Some(p) = &item.processed_artifact_path {
                        std::path::PathBuf::from(p)
                    } else if let Some(p) = &item.local_artifact_path {
                        std::path::PathBuf::from(p)
                    } else {
                        workspace.join(format!("{}", item.id))
                    };
                    let size = std::fs::metadata(&path)
                        .map(|m| m.len() as i64)
                        .unwrap_or(item.size_bytes);
                    (path, size)
                };`
);

code = code.replace(
    /                    filename: item\.name\.clone\(\),/m,
    `                    filename: if media_kind == TelegramMediaKind::Video {
                        let mut name = item.name.clone();
                        if let Some(idx) = name.rfind('.') {
                            name.truncate(idx);
                        }
                        format!("{}.mp4", name)
                    } else {
                        item.name.clone()
                    },`
);

fs.writeFileSync('app/src-tauri/src/migration/pipeline/runner.rs', code);
