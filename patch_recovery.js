const fs = require('fs');
let code = fs.readFileSync('app/src-tauri/src/migration/pipeline/runner.rs', 'utf8');

code = code.replace(
    /                    local_artifact_path: original_path,[\s\S]*?telegram_random_id: None,/m,
    `                    local_artifact_path: original_path,
                    processed_artifact_path: processed_path.clone(),
                    telegram_random_id: None,`
);

code = code.replace(
    /                    let processed_exists = std::path::Path::new\(&workspace\.join\(format\!\("\{}\.processed\.mp4", item\.id\)\)\)\.exists\(\);[\s\S]*?let original_exists = std::path::Path::new\(&workspace\.join\(format\!\("\{}", item\.id\)\)\)\.exists\(\);/m,
    `                    let processed_exists = std::path::Path::new(item.processed_artifact_path.as_deref().unwrap_or("")).exists();
                    let original_exists = std::path::Path::new(item.local_artifact_path.as_deref().unwrap_or("")).exists();`
);

code = code.replace(
    /                "saving_local" => \{[\s\S]*?let original_exists = std::path::Path::new\(&self\.workspace_dir\.join\(format\!\("\{}", item\.id\)\)\)\.exists\(\);/m,
    `                "saving_local" => {
                    let original_exists = std::path::Path::new(item.local_artifact_path.as_deref().unwrap_or("")).exists();`
);

fs.writeFileSync('app/src-tauri/src/migration/pipeline/runner.rs', code);
