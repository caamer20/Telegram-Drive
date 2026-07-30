package com.cameronamer.telegramdrive.nativeplayer

import android.app.Activity
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

@TauriPlugin
class NativePlayerPlugin(private val activity: Activity) : Plugin(activity) {
    @Volatile private var pendingLaunch: NativePlayerLaunchHandle? = null
    @Volatile private var pendingResult: NativePlayerResultData? = null

    @Command
    fun openNativePlayer(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(OpenNativePlayerArgs::class.java)
            args.validate()
            val request = NativePlayerLaunchRequest(
                args.folderId, args.messageId, args.title, args.fileName, args.mimeType,
                args.startPositionMs, args.autoplay,
            )
            val streamPath = "/stream/${args.folderId ?: "home"}/${args.messageId}"
            require(args.streamUrl.endsWith(streamPath)) { "stream identity does not match request" }
            val playbackSession = NativePlaybackSession(
                baseUrl = args.streamUrl.removeSuffix(streamPath),
                authorizationToken = args.authorizationToken,
                codec = args.codec,
                width = args.width,
                height = args.height,
                frameRate = args.frameRate,
                bitrate = args.bitrate,
                bitDepth = args.bitDepth,
                hdr = args.hdr,
            )
            pendingResult = null
            pendingLaunch = NativePlayerLauncher.prepare(
                activity = activity,
                callerKey = CALLER_KEY,
                request = request,
                playbackSession = playbackSession,
                onState = { snapshot -> trigger("native-player://playback-state", snapshot.toJsObject()) },
                callback = object : NativePlayerLaunchResultCallback {
                    override fun onResult(result: NativePlayerResultData) {
                        pendingResult = result
                    }
                },
            )
            startActivityForResult(invoke, pendingLaunch!!.intent, "nativePlayerResult")
        } catch (_: Exception) {
            NativePlayerLauncher.abandon(CALLER_KEY)
            pendingLaunch = null
            invoke.reject("invalid native player request")
        }
    }

    @ActivityCallback
    fun nativePlayerResult(invoke: Invoke, result: ActivityResult) {
        pendingLaunch = null
        val response = NativePlayerLauncher.complete(CALLER_KEY, result.data)
        pendingResult = null
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

    @Command
    fun clearPendingRestore(invoke: Invoke) {
        PendingNativePlayerRestoreStore.clear(activity)
        invoke.resolve()
    }

    companion object {
        private const val CALLER_KEY = "tauri-native-player-plugin"
    }
}
