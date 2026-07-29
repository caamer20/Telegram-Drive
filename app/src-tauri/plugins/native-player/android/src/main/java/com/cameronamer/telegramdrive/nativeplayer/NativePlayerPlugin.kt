package com.cameronamer.telegramdrive.nativeplayer

import android.app.Activity
import android.content.Intent
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.util.concurrent.atomic.AtomicBoolean

@TauriPlugin
class NativePlayerPlugin(private val activity: Activity) : Plugin(activity) {
    private val opening = AtomicBoolean(false)
    @Volatile private var pendingSessionId: String? = null

    @Command
    fun openNativePlayer(invoke: Invoke) {
        if (!opening.compareAndSet(false, true)) {
            invoke.reject("native player is already open")
            return
        }
        try {
            val args = invoke.parseArgs(OpenNativePlayerArgs::class.java)
            args.validate()
            PendingNativePlayerRestoreStore.clear(activity)
            NativePlayerActivityRegistry.clearPendingClose()
            val session = NativePlayerSessionStore.create(args)
            pendingSessionId = session.id
            session.stateListener = { snapshot ->
                trigger("native-player://playback-state", snapshot.toJsObject())
            }
            val intent = Intent(activity, NativePlayerActivity::class.java).apply {
                putExtra(NativePlayerActivity.EXTRA_SESSION_ID, session.id)
                args.folderId?.let { putExtra(NativePlayerActivity.EXTRA_FOLDER_ID, it) }
                putExtra(NativePlayerActivity.EXTRA_MESSAGE_ID, args.messageId)
                putExtra(NativePlayerActivity.EXTRA_TITLE, args.title)
                args.fileName?.let { putExtra(NativePlayerActivity.EXTRA_FILE_NAME, it) }
                args.mimeType?.let { putExtra(NativePlayerActivity.EXTRA_MIME_TYPE, it) }
                putExtra(NativePlayerActivity.EXTRA_AUTOPLAY, args.autoplay)
            }
            startActivityForResult(invoke, intent, "nativePlayerResult")
        } catch (_: Exception) {
            opening.set(false)
            pendingSessionId?.let(NativePlayerSessionStore::remove)
            pendingSessionId = null
            invoke.reject("invalid native player request")
        }
    }

    @ActivityCallback
    fun nativePlayerResult(invoke: Invoke, result: ActivityResult) {
        val sessionId = pendingSessionId
        pendingSessionId = null
        opening.set(false)
        NativePlayerActivityRegistry.clearPendingClose()
        NativePlayerSessionStore.remove(sessionId)
        val response = NativePlayerResultCodec.fromIntent(result.data)
        invoke.resolve(response.toJsObject())
    }

    @Command
    fun closeNativePlayer(invoke: Invoke) {
        activity.runOnUiThread { NativePlayerActivityRegistry.close() }
        invoke.resolve()
    }

    @Command
    fun getNativePlaybackState(invoke: Invoke) {
        invoke.resolve(NativePlayerActivityRegistry.snapshot().toJsObject())
    }

    @Command
    fun takePendingRestore(invoke: Invoke) {
        val restore = PendingNativePlayerRestoreStore.take(activity)
        if (restore == null) invoke.resolve() else invoke.resolve(restore.toJsObject())
    }
}
