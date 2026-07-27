const fs = require('fs');

let media = fs.readFileSync('app/src-tauri/src/migration/adapters/media.rs', 'utf8');
media = media.replace(/\/\/ removed libx264 test/g, '');
fs.writeFileSync('app/src-tauri/src/migration/adapters/media.rs', media);

let factory = fs.readFileSync('app/src-tauri/src/migration/adapters/factory.rs', 'utf8');
factory = factory.replace(/\/\/ CPU threads: min\(2, available_parallelism\)/g, '');
fs.writeFileSync('app/src-tauri/src/migration/adapters/factory.rs', factory);

let runner = fs.readFileSync('app/src-tauri/src/migration/pipeline/runner.rs', 'utf8');
runner = runner.replace(/\/\/ A2 fix: remux_copy ALSO uses \.processed\.mp4, not just transcode/g, '');
fs.writeFileSync('app/src-tauri/src/migration/pipeline/runner.rs', runner);
