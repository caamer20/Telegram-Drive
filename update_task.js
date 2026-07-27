const fs = require('fs');
const taskPath = '/Users/manhtuong/.gemini/antigravity/brain/b45050f6-d136-44b9-b785-51567ae69e8b/task.md';
let task = fs.readFileSync(taskPath, 'utf8');

task = task.replace(/\[\/\] Phân loại HEVC Canonical/, '[x] Phân loại HEVC Canonical');
task = task.replace(/\[\/\] Concurrent Pipeline/, '[x] Concurrent Pipeline');
task = task.replace(/\[\/\] Băm Streaming & Validation/, '[x] Băm Streaming & Validation');
task = task.replace(/\[\/\] Artifact Checkpoint & Quota Worker/, '[x] Artifact Checkpoint & Quota Worker');
task = task.replace(/\[\/\] Frontend UI cho 3 luồng Progress/, '[x] Frontend UI cho 3 luồng Progress');
task = task.replace(/\[ \] Tối ưu hóa UI React Progress Panel/, '[x] Tối ưu hóa UI React Progress Panel');
task = task.replace(/\[ \] Thay \`app\.emit_all\` bằng \`app\.emit\`/g, '[x] Thay `app.emit_all` bằng `app.emit`');

fs.writeFileSync(taskPath, task);
