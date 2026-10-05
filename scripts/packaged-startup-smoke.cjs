#!/usr/bin/env node
// Run only in a disposable OS user/VM. Installer/keyring state is not redirected
// merely by HOME; the app also uses a unique smoke-only runtime identifier.
const fs = require('node:fs'), path = require('node:path'), os = require('node:os');
const { spawn, spawnSync } = require('node:child_process');
const { randomBytes } = require('node:crypto');
async function boundedWait(promise, milliseconds) {
  let timer;
  try { return await Promise.race([promise, new Promise(resolve => { timer = setTimeout(resolve, milliseconds); })]); }
  finally { clearTimeout(timer); }
}
function belongsToLaunch(pid, launched) {
  if (pid === launched) return true;
  if (process.platform === 'win32' || !Number.isSafeInteger(pid) || pid < 2) return false;
  for (let depth = 0; depth < 16; depth++) {
    const parent = spawnSync('ps', ['-o', 'ppid=', '-p', String(pid)], { encoding: 'utf8', timeout: 2000 });
    if (parent.status !== 0) return false;
    pid = Number(parent.stdout.trim()); if (pid === launched) return true; if (pid < 2) return false;
  }
  return false;
}
async function main() {
  const options = { timeout: 45_000, args: [] };
  for (let i = 2; i < process.argv.length; i++) {
    const flag = process.argv[i];
    if (flag === '--disposable-user') options.disposable = true;
    else if (flag === '--executable') options.executable = process.argv[++i];
    else if (flag === '--args-json') options.args = JSON.parse(process.argv[++i]);
    else if (flag === '--timeout-ms') options.timeout = Number(process.argv[++i]);
    else if (flag === '--run-token') options.token = process.argv[++i];
    else if (flag === '--expected-bundle-type') options.bundleType = process.argv[++i];
    else throw new Error(`Unknown flag ${flag}`);
  }
  if (!options.disposable && process.env.GITHUB_ACTIONS !== 'true') throw new Error('Packaged smoke requires a disposable OS user/VM: confirm with --disposable-user');
  if (!Array.isArray(options.args) || options.args.some(value => typeof value !== 'string') || !Number.isInteger(options.timeout) || options.timeout < 100 || options.timeout > 60_000) throw new Error('Invalid smoke arguments/deadline');
  if (options.token !== undefined && !/^[a-fA-F0-9]{32}$/.test(options.token)) throw new Error('Invalid smoke run token');
  if (options.bundleType !== undefined && !['app', 'appimage', 'deb', 'rpm', 'nsis', 'msi'].includes(options.bundleType)) throw new Error('Invalid expected bundle type');
  const executable = fs.realpathSync(options.executable);
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'telegram-drive-packaged-smoke-'));
  fs.chmodSync(root, 0o700);
  // Keep the normal suffix alpha-leading (124 random bits). Explicit numeric
  // nonces are escaped for D-Bus; the authentication token itself is unchanged.
  const token = options.token ?? ('a' + randomBytes(16).toString('hex').slice(1));
  const identifier = `com.cameronamer.telegramdrive.smoke.${/^\d/.test(token) ? 'r' : ''}${token}`;
  fs.writeFileSync(path.join(root, '.packaged-startup-smoke'), `${token}\n`, { mode: 0o600 });
  const environment = { ...process.env, TELEGRAM_DRIVE_STARTUP_SMOKE_ROOT: root, TELEGRAM_DRIVE_STARTUP_SMOKE_TOKEN: token, TELEGRAM_DRIVE_SMOKE_DISPOSABLE_USER: '1',
    XDG_DATA_HOME: path.join(root, 'data'), XDG_CONFIG_HOME: path.join(root, 'config'), XDG_CACHE_HOME: path.join(root, 'cache'), TMPDIR: path.join(root, 'tmp'), TMP: path.join(root, 'tmp'), TEMP: path.join(root, 'tmp') };
  // Windows KnownFolder resolution belongs to the disposable OS account.
  // Replacing its profile variables can make SHGetKnownFolderPath fail.
  if (process.platform !== 'win32') Object.assign(environment, {
    HOME: path.join(root, 'home'), USERPROFILE: path.join(root, 'home'), APPDATA: path.join(root, 'roaming'), LOCALAPPDATA: path.join(root, 'local') });
  for (const directory of ['home', 'roaming', 'local', 'data', 'config', 'cache', 'tmp']) fs.mkdirSync(path.join(root, directory));
  let launchExecutable = executable, launchArgs = options.args;
  const launchRecord = path.join(root, 'windows-launch.json');
  if (process.platform === 'win32') {
    const config = path.join(root, 'windows-command.json');
    fs.writeFileSync(config, JSON.stringify({ executable, args: options.args, record: launchRecord }), { mode: 0o600 });
    launchExecutable = 'pwsh.exe';
    launchArgs = ['-NoLogo', '-NoProfile', '-NonInteractive', '-File', path.join(__dirname, 'smoke-windows-job.ps1'), '-Configuration', config];
  }
  const child = spawn(launchExecutable, launchArgs, { cwd: path.dirname(executable), env: environment, stdio: ['ignore', 'pipe', 'pipe'], detached: process.platform !== 'win32' });
  let output = '', exited = false, ready, failure;
  child.stdout.on('data', data => { output = (output + data).slice(-16_384); }); child.stderr.on('data', data => { output = (output + data).slice(-16_384); });
  const completion = new Promise(resolve => { child.once('error', error => { failure = error; exited = true; resolve(); }); child.once('close', () => { exited = true; resolve(); }); });
  let startupError;
  try {
    const deadline = Date.now() + options.timeout;
    while (Date.now() < deadline && !exited) {
      const marker = path.join(root, 'startup-ready.json');
      if (fs.existsSync(marker)) {
        const value = JSON.parse(fs.readFileSync(marker, 'utf8'));
        const launched = process.platform === 'win32' ? JSON.parse(fs.readFileSync(launchRecord, 'utf8')).process_id : child.pid;
        if (!belongsToLaunch(value.process_id, launched) || value.run_token !== token || value.profile_identifier !== identifier || !value.database_ready || !value.app_data_ready || !value.streaming_runtime_ready) throw new Error('Invalid or stale application readiness marker');
        for (const name of ['app_data_dir', 'app_cache_dir', 'app_config_dir', 'app_local_data_dir']) {
          if (typeof value[name] !== 'string' || path.basename(value[name]) !== identifier) throw new Error(`Readiness ${name} is not the isolated Tauri profile`);
        }
        ready = value;
        if (options.bundleType && value.bundle_type !== options.bundleType) throw new Error(`Runtime bundle type ${value.bundle_type ?? 'unknown'} does not match ${options.bundleType}`);
        break;
      }
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    if (!ready || exited) throw failure || new Error(`Application readiness not reached${exited ? ' before early exit' : ' before deadline'}\n${output}`);
  } catch (error) {
    startupError = error;
    throw error;
  } finally {
    let naturalExit;
    try {
      // Ask the Windows owner to terminate its job and prove it is empty before
      // releasing handles. Unix launches own a separate process group.
      if (child.pid) {
        if (process.platform === 'win32') {
          fs.writeFileSync(`${launchRecord}.stop`, 'stop\n');
          await boundedWait(completion, 12_000);
          if (!exited || !fs.existsSync(`${launchRecord}.reaped`)) {
            if (!exited) spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { encoding: 'utf8', timeout: 10_000 });
            throw new Error('Windows smoke job did not confirm all descendants reaped');
          }
          const outcome = JSON.parse(fs.readFileSync(`${launchRecord}.reaped`, 'utf8'));
          if (typeof outcome.natural_exit !== 'boolean' || !Number.isInteger(outcome.exit_code)) throw new Error('Invalid Windows smoke job outcome');
          naturalExit = outcome.natural_exit;
        } else { try { process.kill(-child.pid, 'SIGTERM'); } catch {} }
      }
      await boundedWait(completion, 2_000);
      if (process.platform !== 'win32' && child.pid) { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
      await boundedWait(completion, 5_000);
      if (!exited) throw new Error('Smoke process could not be reaped');
      // Windows/macOS known-folder APIs may choose locations outside the temporary
      // HOME. Remove only the exact unique profile paths returned by readiness.
      if (ready) for (const name of ['app_data_dir', 'app_cache_dir', 'app_config_dir', 'app_local_data_dir']) {
        const directory = ready[name]; if (path.basename(directory) === identifier) fs.rmSync(directory, { recursive: true, force: true });
      }
      fs.rmSync(root, { recursive: true, force: true });
    } catch (cleanupError) {
      if (!startupError) throw cleanupError;
      console.error(`[packaged-smoke] Cleanup also failed: ${cleanupError.message}`);
    }
    if (!startupError && naturalExit) throw new Error(`Application exited naturally before controlled smoke shutdown\n${output}`);
  }
  console.log(`[packaged-smoke] Startup readiness passed for ${path.basename(executable)} (${ready.version}, bundle ${ready.bundle_type ?? 'unchecked'}), isolated profile ${identifier}.`);
}
main().catch(error => { console.error(`[packaged-smoke] ${error.message}`); process.exitCode = 1; });
