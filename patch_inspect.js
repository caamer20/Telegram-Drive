const fs = require('fs');
let code = fs.readFileSync('app/src-tauri/src/migration/adapters/media.rs', 'utf8');

code = code.replace(
    /let output = runner\n\s*\.run_command\(&ffprobe, &args\)/,
    `let output = runner\n                .run_command(&ffprobe, &args, None)`
);

fs.writeFileSync('app/src-tauri/src/migration/adapters/media.rs', code);
