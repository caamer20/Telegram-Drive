package com.cameronamer.telegramdrive.nativeplayer

import android.content.Context
import android.content.Intent
import app.tauri.plugin.JSObject

data class PendingNativePlayerRestore(
    val folderId: Long?,
    val messageId: Int,
    val title: String,
    val fileName: String?,
    val mimeType: String?,
    val startPositionMs: Long,
    val autoplay: Boolean,
) {
    fun toJsObject(): JSObject = JSObject().apply {
        folderId?.let { put("folderId", it) } ?: put("folderId", null)
        put("messageId", messageId)
        put("title", title)
        fileName?.let { put("fileName", it) }
        mimeType?.let { put("mimeType", it) }
        put("startPositionMs", startPositionMs)
        put("autoplay", autoplay)
    }
}

/** Identity-only process-death handoff. Stream URLs and credentials are never persisted. */
object PendingNativePlayerRestoreStore {
    private const val PREFS = "native-player-restore"

    fun save(
        context: Context,
        intent: Intent,
        positionMs: Long,
        playWhenReady: Boolean,
        nowMs: Long = System.currentTimeMillis(),
    ) {
        val messageId = intent.getIntExtra(NativePlayerActivity.EXTRA_MESSAGE_ID, 0)
        val title = intent.getStringExtra(NativePlayerActivity.EXTRA_TITLE).orEmpty()
        if (messageId <= 0 || title.isBlank() || title.length > 256) return
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .clear()
            .putBoolean("present", true)
            .putLong("createdAtMs", nowMs)
            .putBoolean("hasFolder", intent.hasExtra(NativePlayerActivity.EXTRA_FOLDER_ID))
            .putLong("folderId", intent.getLongExtra(NativePlayerActivity.EXTRA_FOLDER_ID, 0))
            .putInt("messageId", messageId)
            .putString("title", title)
            .putString("fileName", intent.getStringExtra(NativePlayerActivity.EXTRA_FILE_NAME))
            .putString("mimeType", intent.getStringExtra(NativePlayerActivity.EXTRA_MIME_TYPE))
            .putLong("positionMs", positionMs.coerceIn(0, MAX_START_POSITION_MS))
            .putBoolean("autoplay", playWhenReady)
            .apply()
    }

    fun take(context: Context, nowMs: Long = System.currentTimeMillis()): PendingNativePlayerRestore? {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (!prefs.getBoolean("present", false)) return null
        val createdAtMs = prefs.getLong("createdAtMs", 0)
        val restore = PendingNativePlayerRestore(
            folderId = if (prefs.getBoolean("hasFolder", false)) prefs.getLong("folderId", 0) else null,
            messageId = prefs.getInt("messageId", 0),
            title = prefs.getString("title", "").orEmpty(),
            fileName = prefs.getString("fileName", null),
            mimeType = prefs.getString("mimeType", null),
            startPositionMs = prefs.getLong("positionMs", 0).coerceIn(0, MAX_START_POSITION_MS),
            autoplay = prefs.getBoolean("autoplay", true),
        )
        prefs.edit().clear().apply()
        return restore.takeIf { isFreshAndValid(it, createdAtMs, nowMs) }
    }

    fun clear(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().clear().apply()
    }

    internal fun isFreshAndValid(
        restore: PendingNativePlayerRestore,
        createdAtMs: Long,
        nowMs: Long,
    ): Boolean {
        val isFresh = createdAtMs > 0 && nowMs >= createdAtMs && nowMs - createdAtMs <= RESTORE_TTL_MS
        val isValid = restore.messageId > 0 && restore.title.isNotBlank() && restore.title.length <= 256 &&
            (restore.folderId == null || restore.folderId > 0) &&
            restore.fileName?.length?.let { it <= 512 } != false &&
            restore.mimeType?.length?.let { it <= 128 } != false
        return isFresh && isValid
    }

    private const val MAX_START_POSITION_MS = 30L * 24 * 60 * 60 * 1000
    internal const val RESTORE_TTL_MS = 15L * 60 * 1000
}
