package com.cameronamer.telegramdrive.nativeplayer

import android.content.Intent
import app.tauri.annotation.InvokeArg
import app.tauri.plugin.JSObject

@InvokeArg
class OpenNativePlayerArgs {
    var folderId: Long? = null
    var messageId: Int = 0
    lateinit var title: String
    var fileName: String? = null
    var mimeType: String? = null
    var startPositionMs: Long = 0
    var autoplay: Boolean = true
    lateinit var streamUrl: String
    lateinit var authorizationToken: String
    var codec: String? = null
    var width: Int? = null
    var height: Int? = null
    var frameRate: Double? = null
    var bitrate: Long? = null
    var bitDepth: Int? = null
    var hdr: Boolean? = null

    fun validate() {
        require(messageId > 0) { "messageId must be positive" }
        require(folderId == null || folderId!! > 0) { "folderId must be null or positive" }
        require(title.isNotBlank() && title.length <= 256) { "title is invalid" }
        require(fileName == null || fileName!!.length <= 512) { "fileName is too long" }
        require(mimeType == null || mimeType!!.length <= 128) { "mimeType is too long" }
        require(startPositionMs in 0..MAX_START_POSITION_MS) { "startPositionMs is invalid" }
        require(LOOPBACK_STREAM.matches(streamUrl)) { "only trusted loopback streams are allowed" }
        require(authorizationToken.isNotEmpty() && authorizationToken.length <= 512) {
            "stream credentials are invalid"
        }
        listOfNotNull(fileName, mimeType).forEach { value ->
            val lower = value.trim().lowercase()
            require(!lower.contains("://") && !lower.startsWith("file:") && !lower.startsWith("content:")) {
                "arbitrary URIs are not allowed"
            }
        }
    }

    companion object {
        private const val MAX_START_POSITION_MS = 30L * 24 * 60 * 60 * 1000
        private val LOOPBACK_STREAM = Regex("^http://127\\.0\\.0\\.1:[1-9][0-9]{0,4}/stream/(home|[1-9][0-9]*)/[1-9][0-9]*$")
    }
}
data class NativePlayerPublicError(
    val category: String,
    val code: String,
    val message: String,
)

data class NativePlayerResultData(
    val positionMs: Long = 0,
    val durationMs: Long = 0,
    val completed: Boolean = false,
    val exitReason: String = "external",
    val error: NativePlayerPublicError? = null,
) {
    fun toJsObject(): JSObject = JSObject().apply {
        put("positionMs", positionMs)
        put("durationMs", durationMs)
        put("completed", completed)
        put("exitReason", exitReason)
        error?.let { publicError ->
            put("error", JSObject().apply {
                put("category", publicError.category)
                put("code", publicError.code)
                put("message", publicError.message)
            })
        }
    }
}

data class NativePlaybackSnapshot(
    val state: String = "idle",
    val isPlaying: Boolean = false,
    val positionMs: Long = 0,
    val durationMs: Long = 0,
) {
    fun toJsObject(): JSObject = JSObject().apply {
        put("state", state)
        put("isPlaying", isPlaying)
        put("positionMs", positionMs)
        put("durationMs", durationMs)
    }
}

object NativePlayerResultCodec {
    private const val POSITION = "nativePlayer.positionMs"
    private const val DURATION = "nativePlayer.durationMs"
    private const val COMPLETED = "nativePlayer.completed"
    private const val EXIT_REASON = "nativePlayer.exitReason"
    private const val ERROR_CATEGORY = "nativePlayer.error.category"
    private const val ERROR_CODE = "nativePlayer.error.code"
    private const val ERROR_MESSAGE = "nativePlayer.error.message"

    fun toIntent(result: NativePlayerResultData): Intent = Intent().apply {
        putExtra(POSITION, result.positionMs)
        putExtra(DURATION, result.durationMs)
        putExtra(COMPLETED, result.completed)
        putExtra(EXIT_REASON, result.exitReason)
        result.error?.let {
            putExtra(ERROR_CATEGORY, it.category)
            putExtra(ERROR_CODE, it.code)
            putExtra(ERROR_MESSAGE, it.message)
        }
    }

    fun fromIntent(intent: Intent?): NativePlayerResultData {
        if (intent == null) return NativePlayerResultData()
        val category = intent.getStringExtra(ERROR_CATEGORY)
        val error = if (category == null) null else NativePlayerPublicError(
            category,
            intent.getStringExtra(ERROR_CODE) ?: "UNKNOWN",
            intent.getStringExtra(ERROR_MESSAGE) ?: "Playback failed",
        )
        return NativePlayerResultData(
            positionMs = intent.getLongExtra(POSITION, 0).coerceAtLeast(0),
            durationMs = intent.getLongExtra(DURATION, 0).coerceAtLeast(0),
            completed = intent.getBooleanExtra(COMPLETED, false),
            exitReason = intent.getStringExtra(EXIT_REASON) ?: "external",
            error = error,
        )
    }
}
