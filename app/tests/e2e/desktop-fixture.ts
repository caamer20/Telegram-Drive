import type { Page } from '@playwright/test';
import type { SyncPair, SyncStatus } from '../../src/types/sync';

/** Real application entry point and providers; only the native/service boundary is simulated.
 * These browser journeys do not validate SQLite, Keychain, Telegram, or payment cryptography.
 */
export async function desktopFixture(page: Page, options: {
  pickedUploadPaths?: string[];
  syncPairs?: SyncPair[];
  syncStatus?: SyncStatus;
  accountFailures?: number;
  holdFiles?: boolean;
  holdStartupHealth?: boolean;
  holdSupporter?: boolean;
  supporterState?: 'active' | 'needs_refresh' | 'expired' | 'inactive';
  update?: boolean;
  packageManaged?: boolean;
  signedOut?: boolean;
  gatewaySeen?: boolean;
  /** Loopback port reported by the native startup check; defaults to the preferred port. */
  streamingPort?: number;
  sponsorPort?: number;
  /** Extra persisted application settings merged over the fixture defaults. */
  settings?: Record<string, unknown>;
  /** Base64 PDF listed as "Fixture report.pdf" and served through the preview command. */
  pdfBase64?: string;
  heicBase64?: string;
  heicJpegBase64?: string;
  heicDecoderMissing?: boolean;
  video?: boolean;
  extraPhoto?: boolean;
  holdThumbnails?: boolean;
  encryptionReady?: boolean;
  vaultState?: 'none' | 'locked' | 'unlocked';
  populatedTransfers?: boolean;
  /** Persist the controlled native queue and its call history through document reload. */
  persistTransfersAcrossReload?: boolean;
  networkEnabled?: boolean;
  platform?: 'windows' | 'android';
  isTelevision?: boolean;
} = {}) {
  await page.route(`http://localhost:${options.streamingPort ?? 14201}/**`, route => route.fulfill({ status: 200, contentType: 'text/html', body: '<!doctype html><p>Offline sponsor fixture</p>' }));
  if (options.sponsorPort) await page.route(`http://localhost:${options.sponsorPort}/**`, route => route.fulfill({status:200,contentType:"text/html",body:"<!doctype html><p>Offline separate sponsor fixture</p>"}));
  await page.addInitScript(options => {
    const stores = JSON.parse(localStorage.getItem('desktop-e2e-stores') || 'null') ?? {
      'config.json': { api_id: options.signedOut ? undefined : '12345', foldersLastSyncedAt: Date.now(), activeFolderId: 9, ad_gateway_passed: options.gatewaySeen ?? true },
      'settings.json': { settings: { crashReportingEnabled: false, crashReportingConsentSeen: true, driveTourSeen: true, language: 'en', viewMode: 'list', ...(options.settings ?? {}) }, supporter_activation: 'preserve-existing-license' },
    };
    const persist = () => localStorage.setItem('desktop-e2e-stores', JSON.stringify(stores));
    const callbacks = new Map<number, (value: any) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    let nextId = 1;
    let releaseFiles: (() => void) | undefined;
    let releaseSupporter: (() => void) | undefined;
    const fileGate = new Promise<void>(resolve => { releaseFiles = resolve; });
    let releaseThumbnails: (() => void) | undefined;
    const thumbnailGate = new Promise<void>(resolve => { releaseThumbnails = resolve; });
    const supporterGate = new Promise<void>(resolve => { releaseSupporter = resolve; });
    let releaseSearch:(()=>void)|undefined;const searchGate=new Promise<void>(resolve=>{releaseSearch=resolve;});
    let releaseStartupHealth: (()=>void) | undefined;
    const startupHealthGate=new Promise<void>(resolve=>{releaseStartupHealth=resolve;});
    const savedTransfers = options.persistTransfersAcrossReload ? JSON.parse(sessionStorage.getItem('desktop-e2e-native-transfers') || 'null') : null;
    const state = {
      vaultExists:options.vaultState !== undefined && options.vaultState !== 'none',
      jobs: savedTransfers?.jobs ?? (options.populatedTransfers ? [
        {id:'upload-active',direction:'upload',kind:'local_upload',status:'uploading',filename:'Fixture upload.bin',path:'/fixture/Fixture upload.bin'},
        {id:'upload-error',direction:'upload',kind:'local_upload',status:'failed',filename:'Fixture retry.bin',path:'/fixture/Fixture retry.bin',error:'Controlled transport failure'},
        {id:'download-active',direction:'download',kind:'download',status:'downloading',filename:'Fixture download.bin',messageId:42},
        {id:'download-done',direction:'download',kind:'download',status:'completed',filename:'Fixture finished.bin',messageId:43,downloadOutcome:'saved'},
      ].map((job,index)=>({...job,ownerId:'101',folderId:9,progress:40,transferredBytes:400,totalBytes:1000,speedBytesPerSec:128,queuePosition:index,revision:1,createdAt:Date.now(),updatedAt:Date.now()})) as any[] : [] as any[]),
      holdStartupHealth:options.holdStartupHealth??false, sponsorPort:options.sponsorPort ?? options.streamingPort ?? 14201,
      releaseStartupHealth:()=>{state.holdStartupHealth=false;releaseStartupHealth?.();},
      searchHold:false, searchFails:false, searchPageSize:256, searchBuilding:0, searchDuplicateIds:false, releaseSearch:()=>{state.searchHold=false;releaseSearch?.();},
      quotaSaveFails:false, weeklyQuota:Number(localStorage.getItem('desktop-e2e-weekly-quota') ?? 250 * 1024 ** 3),
      owner: '101', accountFailures: options.accountFailures ?? 0, holdFiles: options.holdFiles ?? false,
      holdSupporter: options.holdSupporter ?? false, supporterState: options.supporterState ?? 'active',
      supporterRefreshFails: options.supporterState === 'needs_refresh',
      logoutFails: false, remoteSignOutUnconfirmed: false, saveFails: false, installFails: true, incompleteDownload: false,
      update: options.update ?? false, stores, calls: (savedTransfers?.calls ?? []) as { command: string; args: any }[],
      sync: { settings: { enabled: options.syncStatus?.enabled ?? false, debounceMs: 2000, encryption: 'inherit', scanner: 'full' } as Record<string, unknown>, pairs: options.syncPairs ?? [] as SyncPair[] },
      syncStatus: options.syncStatus,
      holdThumbnails: options.holdThumbnails ?? false, thumbnailVersion: 0,
      releaseThumbnails: () => { state.holdThumbnails = false; releaseThumbnails?.(); },
      inventoryName: null as string | null, inventoryFailure: false, inventoryBuilding: 0, inventoryPresent: true, protectedInterrupted: false, vaultLocked: options.vaultState === 'locked',
      completedFileRequests: [] as { ownerId: string; requestId: string }[],
      releaseFiles: () => { state.holdFiles = false; releaseFiles?.(); },
      releaseSupporter: () => { state.holdSupporter = false; releaseSupporter?.(); },
      emit: (event: string, payload: any) => {
        for (const [id, listener] of listeners) if (listener.event === event) callbacks.get(listener.handler)?.({ id, event, payload });
      },
    };
    const file = (id: number, folder: number | null, owner = state.owner) => ({
      id, folder_id: folder, ownerId: owner, name: `${owner === '101' ? 'Holiday' : 'Work'} ${folder === null ? 'saved' : 'folder'} ${options.heicBase64 ? 'photo.HEIC' : options.video ? 'video.mp4' : 'photo.jpg'}`,
      size: 1024, mime_type: options.heicBase64 ? 'image/heic' : options.video ? 'video/mp4' : 'image/jpeg', file_ext: options.heicBase64 ? 'heic' : options.video ? 'mp4' : 'jpg', created_at: '2026-09-07T12:00:00Z', folderName: folder === null ? 'Saved Messages' : 'Photos',
      encryption_state: 'plain', is_favorite: false, type: 'image',
    });
    const image = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="640" height="480"><rect width="640" height="480" fill="#284b63"/><circle cx="320" cy="240" r="150" fill="#98d8ea"/></svg>')}`;
    const status = () => ({
      state: state.supporterState, ad_free: ['active', 'needs_refresh'].includes(state.supporterState),
      message: 'Fixture entitlement', terms_version: '2026-08-11', terms_url: 'https://example.invalid/terms',
      expires_at: Date.now() / 1000 + (state.supporterState === 'active' ? 86400 : -86400),
      offline_until: Date.now() / 1000 + 6 * 86400, recovery_code_saved: state.supporterState !== 'inactive', checkout_pending: false,
    });
    Object.assign(window, {
      __desktopTest: state,
      __TAURI_OS_PLUGIN_INTERNALS__: { os_type: options.platform ?? 'windows', platform: options.platform ?? 'windows', arch: 'x86_64', version: '11', family: options.platform === 'android' ? 'unix' : 'windows' },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: (id: number) => listeners.delete(id) },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
        transformCallback: (callback: (value: any) => void) => { const id = nextId++; callbacks.set(id, callback); return id; },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        invoke: async (command: string, args: any = {}) => {
          state.calls.push({ command, args });
          try {
          if (command === 'plugin:event|listen') { const id = nextId++; listeners.set(id, { event: args.event, handler: args.handler }); return id; }
          if (command === 'plugin:event|unlisten') { listeners.delete(args.eventId); return; }
          if (command === 'plugin:store|load') { stores[args.path] ??= {}; return args.path; }
          const store = stores[args.rid];
          if (command === 'plugin:store|get') return [store?.[args.key], args.key in (store ?? {})];
          if (command === 'plugin:store|set') { store[args.key] = args.value; return; }
          if (command === 'plugin:store|delete') { delete store[args.key]; return true; }
          if (command === 'plugin:store|save') { if (state.saveFails) throw new Error('Storage unavailable'); persist(); return; }
          if (command === 'cmd_get_settings_sync_status') return {available:true,updated_at:1_790_000_000,device_id:'other-fixture-device',current_device:false};
          if (command === 'cmd_upload_settings_sync') return {available:true,updated_at:1_790_000_001,device_id:'this-fixture-device',current_device:true};
          if (command === 'cmd_get_startup_health') {if(state.holdStartupHealth)await startupHealthGate;return { ready: true, sponsor_port: state.sponsorPort, streaming_port: options.streamingPort ?? 14201, streaming_port_is_fallback: (options.streamingPort ?? 14201) !== 14201 };}
          if (command === 'cmd_auth_qr_login') return 'tg://login?token=browser-fixture-only';
          if (command === 'cmd_auth_qr_poll') return { success: false };
          if (command === 'cmd_auth_request_code') return { status: 'code_required', delivery: 'telegram_app', codeLength: 5, numericCode: true };
          if (command === 'cmd_auth_sign_in') return { success: false, next_step: 'password' };
          if (command === 'cmd_auth_check_password') {
            // Counterfactual response to exercise the defensive UI; current native code never returns false.
            if (args.password === 'unsupported-false-response-fixture') return { success: false };
            // Same message the native command returns for a rejected password.
            if (args.password === 'incorrect-fixture-password') throw new Error('That two-step verification password is incorrect. Check it and try again.');
            return { success: true };
          }
          if (command === 'cmd_get_android_transfer_environment') return {isTelevision: options.isTelevision ?? false};
          if (command === 'cmd_check_connection' || command === 'cmd_is_network_available') return true;
          if (command === 'cmd_search_local') {
            const owner=state.owner;
            if(args.ownerId!==owner)throw new Error('ACCOUNT_CHANGED');
            if(state.searchFails)throw new Error('SEARCH_INDEX_UNAVAILABLE');
            if(state.searchBuilding>0){state.searchBuilding--;throw new Error('INVENTORY_BUILDING');}
            const files=[{...file(42,9),key:'9:42',tags:[],collectionIds:[]}, {...file(state.searchDuplicateIds?42:43,null),key:'saved:43',tags:[],collectionIds:[]}].filter(value=>value.name.toLowerCase().includes(args.query.query.toLowerCase()) && (!args.query.type || args.query.type==='all' || args.query.type==='image'));
            const offset=args.query.offset??0; const limit=Math.min(args.query.limit??256,state.searchPageSize);
            if(state.searchHold)await searchGate;
            return {files:files.slice(offset,offset+limit),total:files.length,indexId:`search-${owner}`,nextOffset:offset+limit<files.length?offset+limit:null,indexed:true,complete:false,offline:true};
          }
          if (command === 'cmd_workspace_account') {
            if (state.accountFailures-- > 0) throw new Error('ACCOUNT_UNAVAILABLE: Session temporarily busy');
            return state.owner;
          }
          if (command === 'cmd_get_enriched_folders' || command === 'cmd_scan_folders') return [{ id: 9, name: 'Photos', file_count: 1 }];
          // Folder Sync: settings and mappings are kept by this fixture; the
          // plan a preview returns is a stand-in for the native planner.
          if (command === 'cmd_get_sync_settings') return state.sync.settings;
          if (command === 'cmd_set_sync_scanner') { state.sync.settings = { ...state.sync.settings, scanner: args.scanner }; return state.sync.settings; }
          if (command === 'cmd_get_sync_pairs') return state.sync.pairs;
          if (command === 'cmd_get_sync_status') return state.syncStatus ?? { enabled: state.sync.settings.enabled, running: false, activePairs: 0, pendingOps: 0, conflicts: 0, lastError: null, pairs: [] };
          if (command === 'plugin:dialog|save') return '/synthetic/Downloads/photo.jpg';
          if (command === 'plugin:dialog|open') return options.pickedUploadPaths ?? '/synthetic/Library';
          if (command === 'cmd_preview_sync_pair') return {
            request: args.request, generatedAt: 1_790_000_000, reviewToken: 'fixture-review-token', accountOwner: args.ownerId, localFiles: 2, remoteFiles: 2,
            counts: { uploads: 0, downloads: 0, deleteLocal: 0, deleteRemote: 0, conflicts: args.request.preferences.adoptMatchingFiles ? 0 : 2, skipped: args.request.preferences.adoptMatchingFiles ? 2 : 0 },
            operations: [], pauseReasons: [], warnings: [],
          };
          if (command === 'cmd_add_sync_pair') {
            const pair = { id: state.sync.pairs.length + 1, localPath: args.localPath, channelId: args.channelId, folderKey: String(args.channelId), label: args.label ?? null, syncDirection: args.syncDirection, isActive: args.isActive, createdAt: 1_790_000_000, accountOwner: args.ownerId, preferences: args.preferences };
            state.sync.pairs.push(pair);
            return pair;
          }
          if (['cmd_get_groups', 'cmd_get_cached_files', 'cmd_get_sync_jobs', 'cmd_list_shares', 'cmd_list_cached_files'].includes(command)) return [];
          if (command === 'cmd_get_file_activity') return [file(42, 9)];
          if (command === 'cmd_touch_file_inventory') return state.inventoryPresent;
          if (command === 'cmd_get_files') {
            state.inventoryPresent = true;
            if (state.inventoryBuilding > 0) { state.inventoryBuilding--; throw new Error('INVENTORY_BUILDING: Reconciliation continues'); }
            if (state.inventoryFailure) throw new Error('INVENTORY_STALE: Verification failed');
            const owner = args.ownerId;
            if (state.protectedInterrupted && !state.vaultLocked) {
              state.emit('folder-load-chunk', { ownerId: owner, folderId: args.folderId, requestId: args.requestId,
                files: [{ ...file(99, args.folderId, owner), name: 'Private decrypted inventory name.pdf', encryption_state: 'encrypted_unlocked' }] });
              await fileGate;
              throw new Error('VAULT_LOCKED: Publication revoked');
            }
            if (state.holdFiles && owner === '101') await fileGate;
            const files = [{ ...file(42, args.folderId, owner), ...(state.inventoryName ? { name: state.inventoryName } : {}) }];
            if (options.extraPhoto) files.push({...file(43,args.folderId,owner),name:'Holiday second photo.jpg'});
            if (options.pdfBase64) files.push({ ...file(77, args.folderId, owner), name: 'Fixture report.pdf', mime_type: 'application/pdf', file_ext: 'pdf', type: 'document' });
            const result = { ownerId: owner, folderId: args.folderId, requestId: args.requestId, files };
            state.emit('folder-load-chunk', result);
            state.completedFileRequests.push({ ownerId: owner, requestId: args.requestId });
            return { ...result, complete: true };
          }
          if (command === 'cmd_get_display_preview' && options.heicBase64) {
            if (options.heicDecoderMissing) throw new Error('HEIC_PREVIEW_UNAVAILABLE');
            return `data:image/jpeg;base64,${options.heicJpegBase64}`;
          }
          if (command === 'cmd_get_preview' && options.heicBase64) return `data:image/heic;base64,${options.heicBase64}`;
          if (command === 'cmd_get_preview' && args.messageId === 77 && options.pdfBase64) return `data:application/pdf;base64,${options.pdfBase64}`;
          if (command === 'cmd_get_stream_info') return { token: 'browser-fixture-token', base_url: `http://localhost:${options.streamingPort ?? 14201}`, operation_token: null };
          if (command === 'cmd_get_thumbnail' && options.video) {
            const owner = state.owner;
            const color = owner === '101' ? state.thumbnailVersion === 0 ? '#284b63' : '#bb8844' : '#994488';
            if (state.holdThumbnails && owner === '101') await thumbnailGate;
            return image.replace(encodeURIComponent('#284b63'), encodeURIComponent(color));
          }
          if (command === 'cmd_get_preview' || command === 'cmd_get_display_preview' || command === 'cmd_workspace_asset') return image;
          if (command === 'cmd_get_bandwidth') return { up_bytes: 0, down_bytes: 0, limit_bytes: state.weeklyQuota, period: 'weekly', date: '2026-09-28' };
          if (command === 'cmd_set_weekly_quota') { if (state.quotaSaveFails) throw new Error('Storage unavailable'); state.weeklyQuota = args.limitBytes; localStorage.setItem('desktop-e2e-weekly-quota', String(args.limitBytes)); return { up_bytes: 0, down_bytes: 0, limit_bytes: state.weeklyQuota, period: 'weekly', date: '2026-09-28' }; }
          if (command === 'cmd_get_storage_insight') return { files: [file(80, 1)], scanned_count: 1200, duplicate_groups: 0, complete: false };
          if (command === 'cmd_get_supporter_status') { if (state.holdSupporter) await supporterGate; return status(); }
          if (command === 'cmd_refresh_supporter') { if (state.supporterRefreshFails) throw new Error('Network offline'); return status(); }
          if (command === 'cmd_begin_supporter_checkout') { if (state.allowCheckoutFixture) return { approval_url: 'https://www.sandbox.paypal.com/checkoutnow?token=PUBLIC-FIXTURE', checkout_id: 'public-fixture' }; throw new Error('Checkout forbidden in browser fixtures'); }
          if (command === 'cmd_activate_supporter') { state.supporterState = 'active'; return status(); }
          if (command === 'cmd_logout') {
            if (state.logoutFails) throw new Error('Private native diagnostic');
            return { signed_out: true, remote_session_revoked: !state.remoteSignOutUnconfirmed };
          }
          if (command === 'cmd_get_encryption_capabilities') return { contract_version: 2, availability: 'ready', vault: true, per_file: true, core_available:options.encryptionReady ?? false, features:{upload:true,read:true,per_file_passphrase:true,recovery:true,share:true,migration:false},readable_formats:[2],writable_formats:[2],supported_suites:[1],blockers:[],vault_backend:'persistent_file',mode_alpha:false,upload_enabled:true,read_enabled:true,share_enabled:true,migration_enabled:false,envelope_version:2,app_version:'fixture',backend_build_id:'fixture' };
          if (command === 'cmd_get_vault_status') return { exists: state.vaultExists, is_unlocked: state.vaultExists && !state.vaultLocked, session_id:state.vaultExists && !state.vaultLocked ? 123 : null,has_recovery:state.vaultExists,created_at:null,vault_id:state.vaultExists && !state.vaultLocked ? 'fixture-vault' : null };
          if (command === 'cmd_get_crypto_inventory') return {entries:[],total_files:0,total_ciphertext_bytes:0,vault_exists:state.vaultExists,experimental_format_quarantined:false};
          if (command === 'cmd_create_vault' || command === 'cmd_unlock_vault' || command === 'cmd_import_vault_recovery') {state.vaultExists=true;state.vaultLocked=false;return 123;}
          if (command === 'cmd_lock_vault') {state.vaultLocked=true;return;}
          if (command === 'cmd_export_vault_recovery') return 'controlled-recovery-bundle';
          if (command === 'cmd_verify_vault_recovery') return {matches_vault:true,missing_profiles:0,complete:true};
          if (command === 'cmd_get_encryption_settings' || command === 'cmd_update_encryption_settings') return args.settings ?? { default_mode: 'none' };
          if (command === 'cmd_get_file_encryption_info') return { state: 'plain', protection_mode: 'none' };
          if (command === 'cmd_transfer_list') return state.jobs;
          if (command === 'cmd_transfer_enqueue_many') return args.requests.map((request: any) => ({
            ...request, status: 'paused', progress: 0, transferredBytes: 0, totalBytes: request.totalBytes ?? 0,
            speedBytesPerSec: 0, queuePosition: 0, revision: 1, createdAt: Date.now(), updatedAt: Date.now(),
          }));
          if (options.populatedTransfers && /^cmd_transfer_(pause|resume|cancel|retry)(_all)?$/.test(command)) {
            const action=command.split('_')[2];
            const targets=state.jobs.filter(job=>job.ownerId===args.ownerId && (args.id ? job.id===args.id : job.direction===args.direction) && (
              action==='pause' ? ['pending','uploading','downloading','encrypting','decrypting','verifying','waiting_for_network','cooldown'].includes(job.status) :
              action==='resume' ? job.status==='paused' : action==='cancel' ? !['completed','failed','cancelled'].includes(job.status) : ['failed','cancelled','waiting_for_unlock'].includes(job.status)
            ));
            for(const job of targets) {job.status=action==='pause'?'paused':action==='cancel'?'cancelled':job.direction==='upload'?'uploading':'downloading';if(action==='retry')job.error=undefined;job.revision++;state.emit('transfer-upserted',{...job});}
            return args.id ? targets[0] : targets;
          }
          if (options.populatedTransfers && command === 'cmd_transfer_clear_terminal') {
            const ids=state.jobs.filter(job=>!job.persistencePending && job.direction===args.direction && job.ownerId===args.ownerId && (job.status==='completed' || args.includeFailedAndCancelled && ['failed','cancelled'].includes(job.status))).map(job=>job.id);
            state.jobs=state.jobs.filter(job=>!ids.includes(job.id));for(const id of ids)state.emit('transfer-removed',id);return ids;
          }
          if (command === 'cmd_get_api_settings') return { enabled: options.networkEnabled ?? false, running: options.networkEnabled ?? false, port: 14201,key_set:true,last_error:null };
          if (command === 'cmd_get_webdav_settings') return { enabled: options.networkEnabled ?? false, running: options.networkEnabled ?? false, port: 14202,supported:true,token_set:true,write_enabled:true,last_error:null };
          if (command === 'cmd_get_offline_cache_status') return { files: [], total_bytes: 0, max_bytes: 1_000_000_000 };
          if (command === 'cmd_get_installation_info') return { managedByPackageManager: options.packageManaged ?? false, packageManager: options.packageManaged ? 'pacman' : null };
          if (command === 'plugin:updater|check') return state.update ? { rid: 100, currentVersion: '3.9.0', version: '3.9.6', body: 'Reliability update' } : null;
          if (command === 'plugin:updater|download') {
            args.onEvent.onmessage({ event: 'Started', data: { contentLength: 4 } });
            args.onEvent.onmessage({ event: 'Progress', data: { chunkLength: state.incompleteDownload ? 2 : 4 } });
            args.onEvent.onmessage({ event: 'Finished' });
            return 101;
          }
          if (command === 'plugin:updater|install') { if (state.installFails) throw new Error('Disk unavailable'); return; }
          return null;
          } finally {
            if (options.persistTransfersAcrossReload) sessionStorage.setItem('desktop-e2e-native-transfers', JSON.stringify({ jobs: state.jobs, calls: state.calls }));
          }
        },
      },
    });
  }, options);
}

export async function nativeCalls(page: Page, command: string) {
  return page.evaluate(command => (window as any).__desktopTest.calls.filter((call: any) => call.command === command), command);
}

export async function openSettings(page: Page) {
  await page.getByRole('button', { name: 'Preferences', exact: true }).click();
  await page.getByRole('menu').getByRole('button', { name: 'Preferences', exact: true }).click();
}
