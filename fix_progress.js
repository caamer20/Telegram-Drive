const fs = require('fs');

// Patch media.rs
let media = fs.readFileSync('app/src-tauri/src/migration/adapters/media.rs', 'utf8');
media = media.replace(
    /        job_id: i64,\n        duration: f64,\n    \) -> Pin/m,
    `        job_id: i64,
        duration: f64,
        item_name: &str,
    ) -> Pin`
);
media = media.replace(
    /        let app_handle = self\.app_handle\.clone\(\);\n        let duration_us = duration \* 1_000_000\.0;/m,
    `        let app_handle = self.app_handle.clone();
        let duration_us = duration * 1_000_000.0;
        let item_name = item_name.to_string();`
);
media = media.replace(
    /                                    "item_id": item_id,\n                                    "job_id": job_id,\n                                    "progress": percent,\n                                    "state": "processing"/m,
    `                                    "item_id": item_id,
                                    "job_id": job_id,
                                    "item_name": item_name,
                                    "percent": percent,
                                    "phase": "processing"`
);
fs.writeFileSync('app/src-tauri/src/migration/adapters/media.rs', media);

// Patch stages.rs
let stages = fs.readFileSync('app/src-tauri/src/migration/pipeline/stages.rs', 'utf8');
stages = stages.replace(
    /        job_id: i64,\n        duration: f64,\n    \) -> Pin/m,
    `        job_id: i64,
        duration: f64,
        item_name: &str,
    ) -> Pin`
);
fs.writeFileSync('app/src-tauri/src/migration/pipeline/stages.rs', stages);

// Patch runner.rs
let runner = fs.readFileSync('app/src-tauri/src/migration/pipeline/runner.rs', 'utf8');
runner = runner.replace(
    /match processor_clone\n\s*\.process_video\(&input_path, &output_path, decision, item\.id, item\.job_id, meta\.duration\)/m,
    `match processor_clone
                                .process_video(&input_path, &output_path, decision, item.id, item.job_id, meta.duration, &item.name)`
);
fs.writeFileSync('app/src-tauri/src/migration/pipeline/runner.rs', runner);
