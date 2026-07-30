package com.cameronamer.telegramdrive.medialibrary.data

import androidx.room.Entity
import androidx.room.Index
import androidx.room.TypeConverter

enum class MediaType { IMAGE, ANIMATED_IMAGE, VIDEO }
enum class ThumbnailStatus { NOT_REQUESTED, LOADING, READY, NO_THUMBNAIL, FAILED }

class MediaTypeConverters {
    @TypeConverter fun mediaTypeToString(value: MediaType): String = value.name
    @TypeConverter fun mediaTypeFromString(value: String): MediaType = MediaType.valueOf(value)
    @TypeConverter fun thumbnailStatusToString(value: ThumbnailStatus): String = value.name
    @TypeConverter fun thumbnailStatusFromString(value: String): ThumbnailStatus = ThumbnailStatus.valueOf(value)
}

@Entity(
    tableName = "telegram_media",
    primaryKeys = ["accountId", "peerId", "messageId"],
    indices = [
        Index(value = ["accountId", "dateEpochSeconds"]),
        Index(value = ["accountId", "mediaType", "dateEpochSeconds"]),
        Index(value = ["accountId", "normalizedName"]),
        Index(value = ["accountId", "sizeBytes"]),
        Index(value = ["accountId", "peerId"]),
        Index(value = ["accountId", "durationSeconds"]),
        Index(value = ["accountId", "extension"]),
        Index(value = ["accountId", "mimeType"]),
    ],
)
data class TelegramMediaEntity(
    val accountId: Long,
    val peerId: Long,
    val messageId: Int,
    val folderId: Long?,
    val peerName: String?,
    val senderId: Long?,
    val displayName: String,
    val originalFilename: String?,
    val normalizedName: String,
    val normalizedSearchText: String,
    val caption: String?,
    val mediaType: MediaType,
    val mimeType: String?,
    val extension: String?,
    val sizeBytes: Long?,
    val dateEpochSeconds: Long,
    val durationSeconds: Int?,
    val width: Int?,
    val height: Int?,
    val thumbnailAvailable: Boolean,
    val thumbnailVariant: String?,
    val thumbnailPath: String?,
    val thumbnailStatus: ThumbnailStatus,
    val lastSyncedAtEpochSeconds: Long,
    val deleted: Boolean,
) {
    val stableKey: String get() = "${accountId}_${peerId}_${messageId}"
}

@Entity(
    tableName = "telegram_media_peer",
    primaryKeys = ["accountId", "peerId"],
    indices = [Index(value = ["accountId", "name"])],
)
data class TelegramMediaPeerEntity(
    val accountId: Long,
    val peerId: Long,
    val folderId: Long?,
    val name: String,
    val kind: String,
    val lastSyncedAtEpochSeconds: Long?,
)

@Entity(
    tableName = "media_sync_state",
    primaryKeys = ["accountId", "peerId"],
)
data class MediaSyncStateEntity(
    val accountId: Long,
    val peerId: Long,
    val nextOffsetMessageId: Int?,
    val newestIndexedMessageId: Int?,
    val oldestIndexedMessageId: Int?,
    val fullSyncCompleted: Boolean,
    val fullSyncStartedAtEpochSeconds: Long?,
    val lastSuccessfulSyncAtEpochSeconds: Long?,
    val lastError: String?,
)

@Entity(
    tableName = "media_playback_state",
    primaryKeys = ["accountId", "peerId", "messageId"],
    indices = [Index(value = ["accountId", "updatedAtEpochSeconds"])],
)
data class MediaPlaybackStateEntity(
    val accountId: Long,
    val peerId: Long,
    val messageId: Int,
    val positionMs: Long,
    val durationMs: Long?,
    val completed: Boolean,
    val updatedAtEpochSeconds: Long,
)
