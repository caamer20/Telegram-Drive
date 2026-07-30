package com.cameronamer.telegramdrive.medialibrary

import android.app.Activity
import android.content.Intent
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaDatabase
import com.cameronamer.telegramdrive.medialibrary.repository.ThumbnailCache
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicBoolean

@TauriPlugin
class MediaLibraryPlugin(private val activity: Activity) : Plugin(activity) {
    private val opening = AtomicBoolean(false)
    private val cleanupScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    @Volatile private var pendingSessionId: String? = null

    @Command
    fun openMediaLibrary(invoke: Invoke) {
        if (!opening.compareAndSet(false, true)) {
            invoke.reject("media library is already open")
            return
        }
        MediaLibraryRuntimeState.opening = true
        try {
            val args = invoke.parseArgs(OpenMediaLibraryArgs::class.java)
            args.validate()
            MediaLibraryActivityRegistry.clearPendingClose()
            val session = MediaLibrarySessionStore.create(args)
            pendingSessionId = session.id
            val intent = Intent(activity, MediaLibraryActivity::class.java).apply {
                putExtra(MediaLibraryActivity.EXTRA_SESSION_ID, session.id)
            }
            startActivityForResult(invoke, intent, "mediaLibraryResult")
        } catch (_: Exception) {
            pendingSessionId?.let(MediaLibrarySessionStore::remove)
            pendingSessionId = null
            opening.set(false)
            MediaLibraryRuntimeState.opening = false
            invoke.reject("invalid media library request")
        }
    }

    @ActivityCallback
    fun mediaLibraryResult(invoke: Invoke, result: ActivityResult) {
        val sessionId = pendingSessionId
        pendingSessionId = null
        opening.set(false)
        MediaLibraryRuntimeState.resetCredentials()
        MediaLibraryActivityRegistry.clearPendingClose()
        MediaLibrarySessionStore.remove(sessionId)
        val data = result.data
        invoke.resolve(
            MediaLibraryResultData(
                exitReason = data?.getStringExtra(MediaLibraryActivity.RESULT_EXIT_REASON) ?: "back",
                accountId = if (data?.hasExtra(MediaLibraryActivity.RESULT_ACCOUNT_ID) == true) {
                    data.getLongExtra(MediaLibraryActivity.RESULT_ACCOUNT_ID, 0).takeIf { it > 0 }
                } else null,
                error = data?.getStringExtra(MediaLibraryActivity.RESULT_ERROR),
            ).toJsObject(),
        )
    }

    @Command
    fun closeMediaLibrary(invoke: Invoke) {
        activity.runOnUiThread { MediaLibraryActivityRegistry.close() }
        invoke.resolve()
    }

    @Command
    fun getMediaLibraryState(invoke: Invoke) {
        invoke.resolve(MediaLibraryRuntimeState.snapshot(MediaLibraryActivityRegistry.isOpen()).toJsObject())
    }

    @Command
    fun clearMediaLibraryData(invoke: Invoke) {
        val args = try {
            invoke.parseArgs(ClearMediaLibraryArgs::class.java)
        } catch (_: Exception) {
            ClearMediaLibraryArgs()
        }
        val accountId = args.accountId
            ?: MediaLibrarySessionStore.get(pendingSessionId)?.accountId
            ?: MediaLibraryRuntimeState.accountId
        activity.runOnUiThread {
            MediaLibraryActivityRegistry.close()
            com.cameronamer.telegramdrive.nativeplayer.NativePlayerLauncher.close()
        }
        cleanupScope.launch {
            try {
                if (accountId != null && accountId > 0) {
                    val database = TelegramMediaDatabase.get(activity.applicationContext)
                    database.withTransactionClearAccount(accountId)
                    ThumbnailCache(activity.applicationContext, database.mediaDao()).clearAccount(accountId)
                }
                MediaLibrarySessionStore.clear()
                MediaLibraryRuntimeState.accountId = null
                MediaLibraryRuntimeState.resetCredentials()
                invoke.resolve()
            } catch (_: Exception) {
                invoke.reject("media library cleanup failed")
            }
        }
    }
}

