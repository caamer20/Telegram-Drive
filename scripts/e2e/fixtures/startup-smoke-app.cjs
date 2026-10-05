// Controlled application boundary for the smoke CLI. Actual Tauri/WebView
// startup and package metadata are exercised separately in packaged CI.
const fs = require('node:fs'), path = require('node:path');
const { spawn } = require('node:child_process');
const [mode, evidence] = process.argv.slice(2);
function descendantReady() {
  try { const pid = Number(fs.readFileSync(`${evidence}.child`, 'utf8')); return Number.isSafeInteger(pid) && pid > 1; }
  catch { return false; }
}
if (mode === 'descendant') {
  const lock = fs.openSync(`${evidence}.lock`, 'w');
  fs.writeFileSync(evidence, String(process.pid));
  setInterval(() => fs.fsyncSync(lock), 100);
} else {
  const root = process.env.TELEGRAM_DRIVE_STARTUP_SMOKE_ROOT;
  const token = process.env.TELEGRAM_DRIVE_STARTUP_SMOKE_TOKEN;
  const identifier = `com.cameronamer.telegramdrive.smoke.${/^\d/.test(token) ? 'r' : ''}${token}`;
  fs.writeFileSync(evidence, JSON.stringify({ root, token, identifier,
    USERPROFILE: process.env.USERPROFILE, APPDATA: process.env.APPDATA, LOCALAPPDATA: process.env.LOCALAPPDATA }));
  if (mode === 'orphan') {
    const child = spawn(process.execPath, [__filename, 'descendant', `${evidence}.child`], { stdio: 'ignore' });
    child.unref();
    const wait = setInterval(() => {
      if (descendantReady()) { clearInterval(wait); console.error('fixture startup failed'); process.exit(7); }
    }, 10);
  } else if (mode === 'deadline') {
    setInterval(() => {}, 100);
  } else {
    const directory = path.join(root, identifier); fs.mkdirSync(directory);
    const record = { process_id: process.pid, run_token: token, profile_identifier: identifier, version: '4.0.0',
      database_ready: true, app_data_ready: true, streaming_runtime_ready: true, bundle_type: mode === 'wrong-kind' ? 'rpm' : 'nsis' };
    for (const name of ['app_data_dir', 'app_cache_dir', 'app_config_dir', 'app_local_data_dir']) record[name] = directory;
    const publish = () => {
      fs.writeFileSync(path.join(root, 'startup-ready.partial'), JSON.stringify(record));
      fs.renameSync(path.join(root, 'startup-ready.partial'), path.join(root, 'startup-ready.json'));
      if (mode === 'ready-exit') process.exit(7);
      setInterval(() => {}, 100);
    };
    if (mode === 'ready-exit') {
      const child = spawn(process.execPath, [__filename, 'descendant', `${evidence}.child`], { stdio: 'ignore' }); child.unref();
      const wait = setInterval(() => { if (descendantReady()) { clearInterval(wait); publish(); } }, 10);
    } else publish();
  }
}
