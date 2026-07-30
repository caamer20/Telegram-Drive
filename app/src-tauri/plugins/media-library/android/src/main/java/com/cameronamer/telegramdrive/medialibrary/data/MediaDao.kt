package com.cameronamer.telegramdrive.medialibrary.data

import androidx.paging.PagingSource
import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import androidx.room.RawQuery
import androidx.sqlite.db.SupportSQLiteQuery

@Dao
interface MediaDao {
    @RawQuery(observedEntities = [TelegramMediaEntity::class])
    fun pagingSource(query: SupportSQLiteQuery): PagingSource<Int, TelegramMediaEntity>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun upsertMedia(items: List<TelegramMediaEntity>)

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun upsertPeers(items: List<TelegramMediaPeerEntity>)

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun upsertSyncState(state: MediaSyncStateEntity)

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun upsertPlaybackState(state: MediaPlaybackStateEntity)

    @Query("SELECT * FROM telegram_media WHERE accountId = :accountId AND peerId = :peerId AND messageId = :messageId LIMIT 1")
    suspend fun media(accountId: Long, peerId: Long, messageId: Int): TelegramMediaEntity?

    @Query("SELECT * FROM telegram_media WHERE accountId = :accountId AND peerId = :peerId AND messageId IN (:messageIds)")
    suspend fun mediaForMessages(accountId: Long, peerId: Long, messageIds: List<Int>): List<TelegramMediaEntity>

    @Query("SELECT * FROM media_sync_state WHERE accountId = :accountId AND peerId = :peerId LIMIT 1")
    suspend fun syncState(accountId: Long, peerId: Long): MediaSyncStateEntity?

    @Query("SELECT * FROM media_playback_state WHERE accountId = :accountId AND peerId = :peerId AND messageId = :messageId LIMIT 1")
    suspend fun playbackState(accountId: Long, peerId: Long, messageId: Int): MediaPlaybackStateEntity?

    @Query("DELETE FROM media_playback_state WHERE accountId = :accountId AND peerId = :peerId AND messageId = :messageId")
    suspend fun deletePlaybackState(accountId: Long, peerId: Long, messageId: Int)

    @Query("SELECT * FROM telegram_media_peer WHERE accountId = :accountId ORDER BY name COLLATE NOCASE ASC, peerId ASC")
    suspend fun peers(accountId: Long): List<TelegramMediaPeerEntity>

    @Query("SELECT MAX(lastSuccessfulSyncAtEpochSeconds) FROM media_sync_state WHERE accountId = :accountId")
    suspend fun lastSuccessfulSync(accountId: Long): Long?

    @Query("SELECT COUNT(*) FROM telegram_media WHERE accountId = :accountId AND deleted = 0")
    suspend fun mediaCount(accountId: Long): Int

    @Query("UPDATE telegram_media SET thumbnailStatus = :status, thumbnailPath = :path WHERE accountId = :accountId AND peerId = :peerId AND messageId = :messageId")
    suspend fun updateThumbnail(accountId: Long, peerId: Long, messageId: Int, status: ThumbnailStatus, path: String?)

    @Query("UPDATE telegram_media SET thumbnailStatus = 'NOT_REQUESTED', thumbnailPath = NULL WHERE thumbnailPath = :path")
    suspend fun markThumbnailEvicted(path: String)

    @Query("UPDATE telegram_media SET deleted = 1 WHERE accountId = :accountId AND peerId = :peerId AND lastSyncedAtEpochSeconds < :syncStartedAt")
    suspend fun markUnseenDeleted(accountId: Long, peerId: Long, syncStartedAt: Long)

    @Query("UPDATE telegram_media SET deleted = 1 WHERE accountId = :accountId AND peerId = :peerId AND messageId BETWEEN :oldestMessageId AND :newestMessageId AND messageId NOT IN (:confirmedMediaIds)")
    suspend fun reconcileMessageWindow(accountId: Long, peerId: Long, oldestMessageId: Int, newestMessageId: Int, confirmedMediaIds: List<Int>)

    @Query("UPDATE telegram_media SET deleted = 1 WHERE accountId = :accountId AND peerId = :peerId AND messageId BETWEEN :oldestMessageId AND :newestMessageId")
    suspend fun reconcileEmptyMessageWindow(accountId: Long, peerId: Long, oldestMessageId: Int, newestMessageId: Int)

    @Query("DELETE FROM telegram_media WHERE accountId = :accountId")
    suspend fun deleteAccountMedia(accountId: Long)

    @Query("DELETE FROM telegram_media_peer WHERE accountId = :accountId")
    suspend fun deleteAccountPeers(accountId: Long)

    @Query("DELETE FROM media_sync_state WHERE accountId = :accountId")
    suspend fun deleteAccountSyncState(accountId: Long)

    @Query("DELETE FROM media_playback_state WHERE accountId = :accountId")
    suspend fun deleteAccountPlayback(accountId: Long)
}
