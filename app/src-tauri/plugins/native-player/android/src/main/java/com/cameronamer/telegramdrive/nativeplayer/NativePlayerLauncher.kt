package com.cameronamer.telegramdrive.nativeplayer

import android.app.Activity
import android.content.Intent
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

data class NativePlayerLaunchRequest(
    val folderId: Long?,
    val messageId: Int,
    val title: String,
    val fileName: String?,
    val mimeType: String?,
    val startPositionMs: Long,
    val autoplay: Boolean,
) {
    fun validate() {
        require(messageId > 0) { "messageId must be positive" }
        require(folderId == null || folderId > 0) { "folderId must be null or positive" }
        require(title.isNotBlank() && title.length <= 256) { "title is invalid" }
        require(fileName == null || fileName.length <= 512) { "fileName is too long" }
        require(mimeType == null || mimeType.length <= 128) { "mimeType is too long" }
        require(startPositionMs in 0..MAX_START_POSITION_MS) { "startPositionMs is invalid" }
        listOfNotNull(fileName, mimeType).forEach { value ->
            val lower = value.trim().lowercase()
            require(!lower.contains("://") && !lower.startsWith("file:") && !lower.startsWith("content:")) {
                "arbitrary URIs are not allowed"
            }
        }
    }

    private companion object {
        const val MAX_START_POSITION_MS = 30L * 24 * 60 * 60 * 1000
    }
}

data class NativePlaybackSession(
    val baseUrl: String,
    val authorizationToken: String,
    val codec: String? = null,
    val width: Int? = null,
    val height: Int? = null,
    val frameRate: Double? = null,
    val bitrate: Long? = null,
    val bitDepth: Int? = null,
    val hdr: Boolean? = null,
) {
    fun validate() {
        require(LOOPBACK_BASE.matches(baseUrl)) { "only trusted IPv4 loopback sessions are allowed" }
        require(authorizationToken.isNotEmpty() && authorizationToken.length <= 512) {
            "stream credentials are invalid"
        }
    }

    private companion object {
        val LOOPBACK_BASE = Regex("^http://127\\.0\\.0\\.1:[1-9][0-9]{0,4}$")
    }
}

interface NativePlayerLaunchResultCallback {
    fun onResult(result: NativePlayerResultData)
}

class NativePlayerLaunchHandle internal constructor(
    internal val callerKey: String,
    val sessionId: String,
    val intent: Intent,
    private val callback: NativePlayerLaunchResultCallback,
) {
    internal fun finish(resultIntent: Intent?) {
        NativePlayerSessionStore.remove(sessionId)
        callback.onResult(NativePlayerResultCodec.fromIntent(resultIntent))
    }
}

class BoundNativePlayerLauncher internal constructor(
    private val activity: ComponentActivity,
    private val callerKey: String,
    private val launchIntent: (Intent) -> Unit,
    private val onState: ((NativePlaybackSnapshot) -> Unit)?,
    private val callback: NativePlayerLaunchResultCallback,
) {
    fun launch(request: NativePlayerLaunchRequest, playbackSession: NativePlaybackSession) {
        val handle = NativePlayerLauncher.prepare(activity, callerKey, request, playbackSession, onState, callback)
        try {
            launchIntent(handle.intent)
        } catch (error: RuntimeException) {
            NativePlayerLauncher.abandon(callerKey)
            throw error
        }
    }
}

/**
 * The only public entry into NativePlayerActivity. Credentials remain in
 * NativePlayerSessionStore and every caller shares validation, duplicate-open,
 * result decoding, cleanup, token lifetime and process-death behavior.
 */
object NativePlayerLauncher {
    private val opening = AtomicBoolean(false)
    private val pending = ConcurrentHashMap<String, NativePlayerLaunchHandle>()

    fun bind(
        activity: ComponentActivity,
        callerKey: String,
        onState: ((NativePlaybackSnapshot) -> Unit)? = null,
        callback: NativePlayerLaunchResultCallback,
    ): BoundNativePlayerLauncher {
        val activityLauncher = activity.registerForActivityResult(
            ActivityResultContracts.StartActivityForResult(),
        ) { result -> complete(callerKey, result.data) }
        return BoundNativePlayerLauncher(activity, callerKey, activityLauncher::launch, onState, callback)
    }

    fun prepare(
        activity: Activity,
        callerKey: String,
        request: NativePlayerLaunchRequest,
        playbackSession: NativePlaybackSession,
        onState: ((NativePlaybackSnapshot) -> Unit)? = null,
        callback: NativePlayerLaunchResultCallback,
    ): NativePlayerLaunchHandle {
        request.validate()
        playbackSession.validate()
        check(opening.compareAndSet(false, true)) { "native player is already open" }
        try {
            check(pending[callerKey] == null) { "native player caller already has a pending launch" }
            val args = OpenNativePlayerArgs().apply {
                folderId = request.folderId
                messageId = request.messageId
                title = request.title
                fileName = request.fileName
                mimeType = request.mimeType
                startPositionMs = request.startPositionMs
                autoplay = request.autoplay
                val folder = request.folderId?.toString() ?: "home"
                streamUrl = "${playbackSession.baseUrl}/stream/$folder/${request.messageId}"
                authorizationToken = playbackSession.authorizationToken
                codec = playbackSession.codec
                width = playbackSession.width
                height = playbackSession.height
                frameRate = playbackSession.frameRate
                bitrate = playbackSession.bitrate
                bitDepth = playbackSession.bitDepth
                hdr = playbackSession.hdr
                validate()
            }
            PendingNativePlayerRestoreStore.clear(activity)
            NativePlayerActivityRegistry.clearPendingClose()
            val playerSession = NativePlayerSessionStore.create(args)
            playerSession.stateListener = onState
            val intent = Intent(activity, NativePlayerActivity::class.java).apply {
                putExtra(NativePlayerActivity.EXTRA_SESSION_ID, playerSession.id)
                request.folderId?.let { putExtra(NativePlayerActivity.EXTRA_FOLDER_ID, it) }
                putExtra(NativePlayerActivity.EXTRA_MESSAGE_ID, request.messageId)
                putExtra(NativePlayerActivity.EXTRA_TITLE, request.title)
                request.fileName?.let { putExtra(NativePlayerActivity.EXTRA_FILE_NAME, it) }
                request.mimeType?.let { putExtra(NativePlayerActivity.EXTRA_MIME_TYPE, it) }
                putExtra(NativePlayerActivity.EXTRA_AUTOPLAY, request.autoplay)
            }
            return NativePlayerLaunchHandle(callerKey, playerSession.id, intent, callback).also {
                pending[callerKey] = it
            }
        } catch (error: RuntimeException) {
            opening.set(false)
            throw error
        }
    }

    fun complete(callerKey: String, resultIntent: Intent?): NativePlayerResultData {
        val handle = pending.remove(callerKey)
        val result = NativePlayerResultCodec.fromIntent(resultIntent)
        try {
            if (handle != null) handle.finish(resultIntent)
        } finally {
            NativePlayerActivityRegistry.clearPendingClose()
            opening.set(false)
        }
        return result
    }

    fun abandon(callerKey: String) {
        pending.remove(callerKey)?.let { NativePlayerSessionStore.remove(it.sessionId) }
        opening.set(false)
    }

    fun close() = NativePlayerActivityRegistry.close()

    internal fun isOpenForTest(): Boolean = opening.get()
    internal fun hasPendingForTest(callerKey: String): Boolean = pending.containsKey(callerKey)

}
