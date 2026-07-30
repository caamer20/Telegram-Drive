package com.cameronamer.telegramdrive.medialibrary

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import androidx.paging.PagingData
import androidx.paging.cachedIn
import com.cameronamer.telegramdrive.medialibrary.data.MediaFilter
import com.cameronamer.telegramdrive.medialibrary.data.MediaSort
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaEntity
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaPeerEntity
import com.cameronamer.telegramdrive.medialibrary.repository.MediaRepository
import com.cameronamer.telegramdrive.medialibrary.repository.SyncProgress
import com.cameronamer.telegramdrive.nativeplayer.NativePlayerLaunchRequest
import com.cameronamer.telegramdrive.nativeplayer.NativePlayerResultData
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import java.io.File

data class RestoredMediaLibraryState(
    val accountId: Long? = null,
    val search: String = "",
    val sort: MediaSort = MediaSort.NEWEST,
    val filter: MediaFilter = MediaFilter(),
    val selectedPeerId: Long? = null,
    val selectedMessageId: Int? = null,
)

data class MediaLibraryUiState(
    val accountId: Long? = null,
    val connecting: Boolean = true,
    val online: Boolean = false,
    val peers: List<TelegramMediaPeerEntity> = emptyList(),
    val lastSuccessfulSync: Long? = null,
    val error: String? = null,
    val playerError: String? = null,
    val selected: TelegramMediaEntity? = null,
)

@OptIn(ExperimentalCoroutinesApi::class)
class MediaLibraryViewModel(
    private val repository: MediaRepository,
    restored: RestoredMediaLibraryState,
) : ViewModel() {
    private val accountId = MutableStateFlow(restored.accountId)
    val search = MutableStateFlow(restored.search)
    val filter = MutableStateFlow(restored.filter)
    val sort = MutableStateFlow(restored.sort)
    private val debouncedSearch = MutableStateFlow(restored.search)
    private val _uiState = MutableStateFlow(MediaLibraryUiState(accountId = restored.accountId))
    val uiState: StateFlow<MediaLibraryUiState> = _uiState.asStateFlow()
    val progress: StateFlow<SyncProgress> = repository.progress
    private val restorePeer = restored.selectedPeerId
    private val restoreMessage = restored.selectedMessageId

    val online: StateFlow<Boolean> = repository.online.stateIn(
        viewModelScope,
        SharingStarted.Eagerly,
        restored.accountId != null,
    )

    val pagingData: Flow<PagingData<TelegramMediaEntity>> = combine(
        accountId,
        debouncedSearch,
        filter,
        sort,
    ) { account, query, activeFilter, activeSort ->
        QueryState(account, query, activeFilter, activeSort)
    }.distinctUntilChanged().flatMapLatest { state ->
        state.accountId?.let { repository.paging(it, state.search, state.filter, state.sort) }
            ?: flowOf(PagingData.empty())
    }.cachedIn(viewModelScope)

    init {
        viewModelScope.launch {
            search.collect { value ->
                delay(250)
                if (search.value == value) debouncedSearch.value = value
            }
        }
        viewModelScope.launch {
            repository.online.collect { isOnline ->
                _uiState.value = _uiState.value.copy(online = isOnline)
            }
        }
        viewModelScope.launch { initialize() }
    }

    private suspend fun initialize() {
        val resolved = repository.connectAccount() ?: accountId.value
        accountId.value = resolved
        if (resolved == null) {
            _uiState.value = _uiState.value.copy(
                connecting = false,
                online = false,
                error = "Reconnect the main Telegram runtime to synchronize and play videos.",
            )
            return
        }
        MediaLibraryRuntimeState.accountId = resolved
        val peers = repository.peers(resolved)
        val lastSync = repository.lastSuccessfulSync(resolved)
        val restoredMedia = if (restorePeer != null && restoreMessage != null) {
            repository.media(resolved, restorePeer, restoreMessage)
        } else null
        _uiState.value = _uiState.value.copy(
            accountId = resolved,
            connecting = false,
            online = repository.online.value,
            peers = peers,
            lastSuccessfulSync = lastSync,
            selected = restoredMedia,
            error = if (repository.online.value) null
                else "Offline library. Reconnect the main Telegram runtime to synchronize and play videos.",
        )
        if (repository.online.value) {
            repository.synchronize(resolved)
            refreshLocalSummaryAfterSync(resolved)
        }
    }

    private fun refreshLocalSummaryAfterSync(account: Long) {
        viewModelScope.launch {
            repository.progress.collect { sync ->
                if (!sync.fullSyncRunning && !sync.incrementalRefreshRunning && !sync.cancellationAvailable) {
                    _uiState.value = _uiState.value.copy(
                        peers = repository.peers(account),
                        lastSuccessfulSync = repository.lastSuccessfulSync(account),
                        error = sync.lastError,
                    )
                }
            }
        }
    }

    fun setFilter(value: MediaFilter) { filter.value = value }
    fun setSort(value: MediaSort) { sort.value = value }
    fun select(item: TelegramMediaEntity?) { _uiState.value = _uiState.value.copy(selected = item, playerError = null) }

    fun refresh(forceFull: Boolean = false) {
        val account = accountId.value ?: return
        if (!repository.online.value) return
        if (forceFull) repository.fullResync(account) else repository.synchronize(account)
    }

    fun cancelSync() = repository.cancelSynchronization()
    fun retrySync() { accountId.value?.let(repository::retry) }

    suspend fun ensureThumbnail(item: TelegramMediaEntity, targetPx: Int = 320, retry: Boolean = false) =
        repository.thumbnailRepository.ensureThumbnail(item, retry, targetPx)

    fun retainThumbnail(file: File) = repository.thumbnailRepository.retain(file)
    fun releaseThumbnail(file: File) = repository.thumbnailRepository.release(file)

    suspend fun playerRequest(item: TelegramMediaEntity): NativePlayerLaunchRequest = repository.playerRequest(item)

    fun acceptPlayerResult(item: TelegramMediaEntity, result: NativePlayerResultData) {
        viewModelScope.launch {
            repository.savePlaybackResult(item, result)
            _uiState.value = _uiState.value.copy(
                selected = item,
                playerError = result.error?.message?.takeIf { !result.errorPresented },
            )
        }
    }

    fun restoredState(): RestoredMediaLibraryState {
        val selected = _uiState.value.selected
        return RestoredMediaLibraryState(
            accountId.value,
            search.value,
            sort.value,
            filter.value,
            selected?.peerId,
            selected?.messageId,
        )
    }

    override fun onCleared() {
        repository.close()
    }

    private data class QueryState(
        val accountId: Long?,
        val search: String,
        val filter: MediaFilter,
        val sort: MediaSort,
    )

    class Factory(
        private val repository: MediaRepository,
        private val restored: RestoredMediaLibraryState,
    ) : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T =
            MediaLibraryViewModel(repository, restored) as T
    }
}
