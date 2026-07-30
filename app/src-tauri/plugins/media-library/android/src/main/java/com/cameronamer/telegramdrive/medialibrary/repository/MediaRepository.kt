package com.cameronamer.telegramdrive.medialibrary.repository

import androidx.paging.Pager
import androidx.paging.PagingConfig
import androidx.paging.PagingData
import androidx.room.withTransaction
import com.cameronamer.telegramdrive.medialibrary.MediaLibraryRuntimeState
import com.cameronamer.telegramdrive.medialibrary.MediaLibrarySession
import com.cameronamer.telegramdrive.medialibrary.data.MediaDao
import com.cameronamer.telegramdrive.medialibrary.data.MediaFilter
import com.cameronamer.telegramdrive.medialibrary.data.MediaPlaybackStateEntity
import com.cameronamer.telegramdrive.medialibrary.data.MediaQueryBuilder
import com.cameronamer.telegramdrive.medialibrary.data.MediaSort
import com.cameronamer.telegramdrive.medialibrary.data.MediaSyncStateEntity
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaDatabase
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaEntity
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaPeerEntity
import com.cameronamer.telegramdrive.medialibrary.network.MediaApiException
import com.cameronamer.telegramdrive.medialibrary.network.MediaLibraryApi
import com.cameronamer.telegramdrive.nativeplayer.NativePlayerLaunchRequest
import com.cameronamer.telegramdrive.nativeplayer.NativePlayerResultData
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlin.math.max
import kotlin.math.min

data class SyncProgress(
    val currentPeerId: Long? = null,
    val currentPeerName: String? = null,
    val completedPeers: Int = 0,
    val totalPeers: Int = 0,
    val messagesScanned: Long = 0,
    val mediaIndexed: Long = 0,
    val fullSyncRunning: Boolean = false,
    val incrementalRefreshRunning: Boolean = false,
    val lastError: String? = null,
    val cancellationAvailable: Boolean = false,
)

class MediaRepository(
    private val database: TelegramMediaDatabase,
    private val session: MediaLibrarySession?,
    private val api: MediaLibraryApi? = session?.let { MediaLibraryApi(it.baseUrl, it.authorizationToken) },
    val thumbnailRepository: ThumbnailRepository,
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO),
) {
    private val dao: MediaDao = database.mediaDao()
    private val _progress = MutableStateFlow(SyncProgress())
    val progress: StateFlow<SyncProgress> = _progress.asStateFlow()
    private val _online = MutableStateFlow(api != null)
    val online: StateFlow<Boolean> = _online.asStateFlow()
    private var syncJob: Job? = null
    @Volatile private var lastAccountId: Long? = session?.accountId

    fun paging(accountId: Long, search: String, filter: MediaFilter, sort: MediaSort): Flow<PagingData<TelegramMediaEntity>> =
        Pager(
            PagingConfig(pageSize = 60, prefetchDistance = 20, enablePlaceholders = false, initialLoadSize = 90),
        ) { dao.pagingSource(MediaQueryBuilder.build(accountId, search, filter, sort)) }.flow

    suspend fun connectAccount(): Long? {
        if (api == null) return lastAccountId
        return try {
            val account = api.account()
            session?.accountId = account.accountId
            lastAccountId = account.accountId
            MediaLibraryRuntimeState.accountId = account.accountId
            MediaLibraryRuntimeState.online = true
            _online.value = true
            account.accountId
        } catch (_: Exception) {
            _online.value = false
            MediaLibraryRuntimeState.online = false
            lastAccountId
        }
    }

    fun synchronize(accountId: Long, forceFull: Boolean = false) {
        if (api == null || syncJob?.isActive == true) return
        syncJob = scope.launch {
            MediaLibraryRuntimeState.syncRunning = true
            try {
                synchronizeInternal(accountId, forceFull)
            } finally {
                MediaLibraryRuntimeState.syncRunning = false
                _progress.value = _progress.value.copy(
                    fullSyncRunning = false,
                    incrementalRefreshRunning = false,
                    cancellationAvailable = false,
                    currentPeerId = null,
                    currentPeerName = null,
                )
            }
        }
    }

    fun cancelSynchronization() {
        syncJob?.cancel()
        syncJob = null
    }

    fun close() {
        cancelSynchronization()
        thumbnailRepository.cancelAll()
    }

    fun retry(accountId: Long) = synchronize(accountId, forceFull = false)
    fun fullResync(accountId: Long) = synchronize(accountId, forceFull = true)

    private suspend fun synchronizeInternal(accountId: Long, forceFull: Boolean) {
        val remotePeers = try {
            api!!.peers()
        } catch (error: Exception) {
            _progress.value = _progress.value.copy(lastError = safeMessage(error))
            handleApiError(error)
            return
        }
        dao.upsertPeers(remotePeers.map {
            TelegramMediaPeerEntity(accountId, it.peerId, it.folderId, it.name, it.kind, null)
        })
        var scannedTotal = 0L
        var indexedTotal = 0L
        var completed = 0
        _progress.value = SyncProgress(totalPeers = remotePeers.size, cancellationAvailable = true)
        for (peer in remotePeers) {
            var state = dao.syncState(accountId, peer.peerId)
            val needsFull = forceFull || state?.fullSyncCompleted != true
            _progress.value = _progress.value.copy(
                currentPeerId = peer.peerId,
                currentPeerName = peer.name,
                completedPeers = completed,
                fullSyncRunning = needsFull,
                incrementalRefreshRunning = !needsFull,
                lastError = null,
            )
            try {
                if (needsFull) {
                    state = fullSyncPeer(accountId, peer.peerId, peer.folderId, state, forceFull) { scanned, indexed ->
                        scannedTotal += scanned
                        indexedTotal += indexed
                        _progress.value = _progress.value.copy(messagesScanned = scannedTotal, mediaIndexed = indexedTotal)
                    }
                } else {
                    state = incrementalSyncPeer(accountId, peer.peerId, peer.folderId, state!!) { scanned, indexed ->
                        scannedTotal += scanned
                        indexedTotal += indexed
                        _progress.value = _progress.value.copy(messagesScanned = scannedTotal, mediaIndexed = indexedTotal)
                    }
                }
                completed += 1
                _progress.value = _progress.value.copy(completedPeers = completed)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                val safe = safeMessage(error)
                // fullSyncPeer commits each page before requesting the next one.
                // Re-read that committed cursor so a later request failure cannot
                // overwrite resumable progress with the pre-loop state.
                state = dao.syncState(accountId, peer.peerId)
                    ?: state
                    ?: emptySyncState(accountId, peer.peerId)
                dao.upsertSyncState(state.copy(lastError = safe))
                _progress.value = _progress.value.copy(lastError = safe)
                handleApiError(error)
            }
        }
    }

    private suspend fun fullSyncPeer(
        accountId: Long,
        peerId: Long,
        folderId: Long?,
        previous: MediaSyncStateEntity?,
        forceFull: Boolean,
        onPage: (Long, Long) -> Unit,
    ): MediaSyncStateEntity {
        val newGeneration = max(now(), (previous?.lastSuccessfulSyncAtEpochSeconds ?: 0) + 1)
        val startedAt = if (forceFull || previous?.fullSyncStartedAtEpochSeconds == null) newGeneration
            else previous.fullSyncStartedAtEpochSeconds
        var state = if (forceFull) {
            emptySyncState(accountId, peerId).copy(fullSyncStartedAtEpochSeconds = startedAt)
        } else {
            (previous ?: emptySyncState(accountId, peerId)).copy(fullSyncStartedAtEpochSeconds = startedAt)
        }
        // Persist the new generation before page one so cancellation or a
        // startup failure cannot silently revert an explicit full resync to
        // the previously completed incremental cursor.
        dao.upsertSyncState(state)
        var offset = state.nextOffsetMessageId ?: 0
        while (true) {
            val page = api!!.mediaPage(folderId, offset)
            state = commitPage(accountId, peerId, page, state, startedAt)
            onPage(page.messagesScanned.toLong(), page.mediaFound.toLong())
            if (!page.hasMore || page.nextOffsetMessageId == null || page.nextOffsetMessageId == offset) {
                val completed = state.copy(
                    nextOffsetMessageId = null,
                    fullSyncCompleted = true,
                    lastSuccessfulSyncAtEpochSeconds = now(),
                    lastError = null,
                )
                database.withTransaction {
                    dao.upsertSyncState(completed)
                    dao.markUnseenDeleted(accountId, peerId, startedAt)
                }
                return completed
            }
            offset = page.nextOffsetMessageId
        }
    }

    private suspend fun incrementalSyncPeer(
        accountId: Long,
        peerId: Long,
        folderId: Long?,
        previous: MediaSyncStateEntity,
        onPage: (Long, Long) -> Unit,
    ): MediaSyncStateEntity {
        val boundary = previous.newestIndexedMessageId
        var offset = 0
        var state = previous
        while (boundary != null) {
            val page = api!!.mediaPage(folderId, offset, newerThanMessageId = boundary)
            state = commitPage(accountId, peerId, page, state, now(), reconcile = true)
            onPage(page.messagesScanned.toLong(), page.mediaFound.toLong())
            if (!page.hasMore || page.reachedNewerThanBoundary || page.nextOffsetMessageId == null || page.nextOffsetMessageId == offset) break
            offset = page.nextOffsetMessageId
        }
        // Re-read a bounded recent window so edited captions/names and reliably
        // absent recent media are reconciled without scanning complete history.
        val recent = api!!.mediaPage(folderId, 0, limit = 200)
        state = commitPage(accountId, peerId, recent, state, now(), reconcile = true)
        onPage(recent.messagesScanned.toLong(), recent.mediaFound.toLong())
        val success = state.copy(lastSuccessfulSyncAtEpochSeconds = now(), lastError = null)
        dao.upsertSyncState(success)
        return success
    }

    private suspend fun commitPage(
        accountId: Long,
        peerId: Long,
        page: com.cameronamer.telegramdrive.medialibrary.network.MediaPageDto,
        previous: MediaSyncStateEntity,
        syncTimestamp: Long,
        reconcile: Boolean = false,
    ): MediaSyncStateEntity {
        check(page.items.all { it.accountId == accountId && it.peerId == peerId }) {
            "media page identity changed"
        }
        val ids = page.items.map { it.messageId }
        val existing = if (ids.isEmpty()) emptyMap() else {
            dao.mediaForMessages(accountId, peerId, ids).associateBy { it.messageId }
        }
        val entities = page.items.map { it.toEntity(syncTimestamp, existing[it.messageId]) }
        val newest = listOfNotNull(previous.newestIndexedMessageId, entities.maxOfOrNull { it.messageId }).maxOrNull()
        val oldest = listOfNotNull(previous.oldestIndexedMessageId, entities.minOfOrNull { it.messageId }).minOrNull()
        val next = previous.copy(
            nextOffsetMessageId = page.nextOffsetMessageId,
            newestIndexedMessageId = newest,
            oldestIndexedMessageId = oldest,
            lastError = null,
        )
        database.withTransaction {
            if (entities.isNotEmpty()) dao.upsertMedia(entities)
            dao.upsertSyncState(next)
            if (reconcile && page.oldestScannedMessageId != null && page.newestScannedMessageId != null) {
                if (ids.isEmpty()) {
                    dao.reconcileEmptyMessageWindow(
                        accountId, peerId, page.oldestScannedMessageId, page.newestScannedMessageId,
                    )
                } else {
                    dao.reconcileMessageWindow(
                        accountId, peerId, page.oldestScannedMessageId, page.newestScannedMessageId, ids,
                    )
                }
            }
        }
        return next
    }

    suspend fun savePlaybackResult(item: TelegramMediaEntity, result: NativePlayerResultData) {
        if (result.completed) {
            dao.deletePlaybackState(item.accountId, item.peerId, item.messageId)
            return
        }
        dao.upsertPlaybackState(
            MediaPlaybackStateEntity(
                item.accountId,
                item.peerId,
                item.messageId,
                result.positionMs.coerceAtLeast(0),
                result.durationMs.takeIf { it > 0 },
                false,
                now(),
            ),
        )
    }

    suspend fun playerRequest(item: TelegramMediaEntity): NativePlayerLaunchRequest {
        val playback = dao.playbackState(item.accountId, item.peerId, item.messageId)
        return NativePlayerLaunchRequest(
            folderId = item.folderId,
            messageId = item.messageId,
            title = item.displayName,
            fileName = item.originalFilename ?: item.displayName,
            mimeType = item.mimeType,
            startPositionMs = playback?.positionMs ?: 0,
            autoplay = true,
        )
    }

    suspend fun peers(accountId: Long): List<TelegramMediaPeerEntity> = dao.peers(accountId)
    suspend fun media(accountId: Long, peerId: Long, messageId: Int): TelegramMediaEntity? =
        dao.media(accountId, peerId, messageId)
    suspend fun lastSuccessfulSync(accountId: Long): Long? = dao.lastSuccessfulSync(accountId)
    suspend fun mediaCount(accountId: Long): Int = dao.mediaCount(accountId)

    private fun emptySyncState(accountId: Long, peerId: Long) = MediaSyncStateEntity(
        accountId, peerId, null, null, null, false, null, null, null,
    )

    private fun handleApiError(error: Exception) {
        if (error is MediaApiException.SessionExpired || error is MediaApiException.RuntimeUnavailable) {
            _online.value = false
            MediaLibraryRuntimeState.online = false
        }
    }

    private fun safeMessage(error: Exception): String = when (error) {
        is MediaApiException.SessionExpired -> "Private media session expired"
        is MediaApiException.RuntimeUnavailable -> "Telegram runtime unavailable"
        is CancellationException -> "Synchronization cancelled"
        else -> "Media synchronization failed"
    }

    private fun now(): Long = System.currentTimeMillis() / 1_000
}
