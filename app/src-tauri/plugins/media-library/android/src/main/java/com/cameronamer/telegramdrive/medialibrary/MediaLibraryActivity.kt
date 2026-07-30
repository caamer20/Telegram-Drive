package com.cameronamer.telegramdrive.medialibrary

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.lifecycle.lifecycleScope
import com.cameronamer.telegramdrive.medialibrary.data.MediaSort
import com.cameronamer.telegramdrive.medialibrary.data.MediaFilter
import com.cameronamer.telegramdrive.medialibrary.data.MediaScope
import com.cameronamer.telegramdrive.medialibrary.data.ResolutionFilter
import com.cameronamer.telegramdrive.medialibrary.data.ThumbnailFilter
import com.cameronamer.telegramdrive.medialibrary.data.MediaType
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaDatabase
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaEntity
import com.cameronamer.telegramdrive.medialibrary.network.MediaLibraryApi
import com.cameronamer.telegramdrive.medialibrary.repository.MediaRepository
import com.cameronamer.telegramdrive.medialibrary.repository.ThumbnailCache
import com.cameronamer.telegramdrive.medialibrary.repository.ThumbnailRepository
import com.cameronamer.telegramdrive.nativeplayer.NativePlaybackSession
import com.cameronamer.telegramdrive.nativeplayer.NativePlayerLaunchResultCallback
import com.cameronamer.telegramdrive.nativeplayer.NativePlayerLauncher
import com.cameronamer.telegramdrive.nativeplayer.NativePlayerResultData
import kotlinx.coroutines.launch

class MediaLibraryActivity : ComponentActivity() {
    private lateinit var repository: MediaRepository
    private lateinit var viewModelFactory: MediaLibraryViewModel.Factory
    private val viewModel: MediaLibraryViewModel by viewModels { viewModelFactory }
    private var pendingPlaybackItem: TelegramMediaEntity? = null
    private var resultSent = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val session = MediaLibrarySessionStore.get(intent.getStringExtra(EXTRA_SESSION_ID))
        val database = TelegramMediaDatabase.get(applicationContext)
        val api = session?.let { MediaLibraryApi(it.baseUrl, it.authorizationToken) }
        val thumbnailCache = ThumbnailCache(applicationContext, database.mediaDao())
        repository = MediaRepository(
            database = database,
            session = session,
            api = api,
            thumbnailRepository = ThumbnailRepository(api, database.mediaDao(), thumbnailCache),
        )
        val restored = RestoredMediaLibraryState(
            accountId = savedInstanceState?.getLong(STATE_ACCOUNT_ID)?.takeIf { it > 0 }
                ?: session?.accountId,
            search = savedInstanceState?.getString(STATE_SEARCH).orEmpty(),
            sort = savedInstanceState?.getString(STATE_SORT)?.let {
                runCatching { MediaSort.valueOf(it) }.getOrNull()
            } ?: MediaSort.NEWEST,
            filter = restoreFilter(savedInstanceState),
            selectedPeerId = savedInstanceState?.getLong(STATE_SELECTED_PEER)?.takeIf { it != 0L },
            selectedMessageId = savedInstanceState?.getInt(STATE_SELECTED_MESSAGE)?.takeIf { it > 0 },
        )
        viewModelFactory = MediaLibraryViewModel.Factory(repository, restored)

        val playerLauncher = NativePlayerLauncher.bind(
            activity = this,
            callerKey = "media-library",
            callback = object : NativePlayerLaunchResultCallback {
                override fun onResult(result: NativePlayerResultData) {
                    pendingPlaybackItem?.let { viewModel.acceptPlayerResult(it, result) }
                    pendingPlaybackItem = null
                }
            },
        )

        MediaLibraryActivityRegistry.register(this)
        MediaLibraryRuntimeState.opening = false
        setContent {
            MediaLibraryTheme {
                MediaLibraryScreen(
                    viewModel = viewModel,
                    onClose = { finishWithResult("back") },
                    onPlayVideo = { item ->
                        if (session == null || item.mediaType != MediaType.VIDEO) return@MediaLibraryScreen
                        lifecycleScope.launch {
                            try {
                                val request = viewModel.playerRequest(item)
                                pendingPlaybackItem = item
                                playerLauncher.launch(
                                    request,
                                    NativePlaybackSession(session.baseUrl, session.authorizationToken),
                                )
                            } catch (_: Exception) {
                                pendingPlaybackItem = null
                                viewModel.acceptPlayerResult(
                                    item,
                                    NativePlayerResultData(
                                        exitReason = "launch-error",
                                        error = com.cameronamer.telegramdrive.nativeplayer.NativePlayerPublicError(
                                            "launcher", "LAUNCH_FAILED", "Unable to open the native player",
                                        ),
                                        errorPresented = false,
                                    ),
                                )
                            }
                        }
                    },
                )
            }
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        val state = viewModel.restoredState()
        state.accountId?.let { outState.putLong(STATE_ACCOUNT_ID, it) }
        outState.putString(STATE_SEARCH, state.search)
        outState.putString(STATE_SORT, state.sort.name)
        saveFilter(outState, state.filter)
        state.selectedPeerId?.let { outState.putLong(STATE_SELECTED_PEER, it) }
        state.selectedMessageId?.let { outState.putInt(STATE_SELECTED_MESSAGE, it) }
        super.onSaveInstanceState(outState)
    }

    override fun onDestroy() {
        if (isFinishing) repository.close()
        MediaLibraryActivityRegistry.clear(this)
        super.onDestroy()
    }

    override fun finish() {
        if (!resultSent) finishWithResult("back") else super.finish()
    }

    fun finishFromExternal() {
        NativePlayerLauncher.close()
        finishWithResult("closed")
    }

    private fun finishWithResult(reason: String) {
        if (resultSent) return
        resultSent = true
        repository.close()
        val account = viewModel.restoredState().accountId
        setResult(
            Activity.RESULT_OK,
            Intent().apply {
                putExtra(RESULT_EXIT_REASON, reason)
                account?.let { putExtra(RESULT_ACCOUNT_ID, it) }
            },
        )
        super.finish()
    }

    private fun restoreFilter(state: Bundle?): MediaFilter {
        if (state == null) return MediaFilter()
        fun <T : Enum<T>> enumValue(key: String, fallback: T, values: Array<T>): T =
            state.getString(key)?.let { name -> values.firstOrNull { it.name == name } } ?: fallback
        return MediaFilter(
            scope = enumValue(STATE_FILTER_SCOPE, MediaScope.ALL, MediaScope.entries.toTypedArray()),
            peerId = state.getLong(STATE_FILTER_PEER).takeIf { state.containsKey(STATE_FILTER_PEER) },
            dateFromEpochSeconds = state.getLong(STATE_FILTER_DATE_FROM).takeIf { state.containsKey(STATE_FILTER_DATE_FROM) },
            dateToEpochSeconds = state.getLong(STATE_FILTER_DATE_TO).takeIf { state.containsKey(STATE_FILTER_DATE_TO) },
            minimumSizeBytes = state.getLong(STATE_FILTER_MIN_SIZE).takeIf { state.containsKey(STATE_FILTER_MIN_SIZE) },
            maximumSizeBytes = state.getLong(STATE_FILTER_MAX_SIZE).takeIf { state.containsKey(STATE_FILTER_MAX_SIZE) },
            minimumDurationSeconds = state.getInt(STATE_FILTER_MIN_DURATION).takeIf { state.containsKey(STATE_FILTER_MIN_DURATION) },
            maximumDurationSeconds = state.getInt(STATE_FILTER_MAX_DURATION).takeIf { state.containsKey(STATE_FILTER_MAX_DURATION) },
            extension = state.getString(STATE_FILTER_EXTENSION),
            mimeType = state.getString(STATE_FILTER_MIME),
            thumbnail = enumValue(STATE_FILTER_THUMBNAIL, ThumbnailFilter.ANY, ThumbnailFilter.entries.toTypedArray()),
            resolution = enumValue(STATE_FILTER_RESOLUTION, ResolutionFilter.ANY, ResolutionFilter.entries.toTypedArray()),
        )
    }

    private fun saveFilter(state: Bundle, filter: MediaFilter) {
        state.putString(STATE_FILTER_SCOPE, filter.scope.name)
        filter.peerId?.let { state.putLong(STATE_FILTER_PEER, it) }
        filter.dateFromEpochSeconds?.let { state.putLong(STATE_FILTER_DATE_FROM, it) }
        filter.dateToEpochSeconds?.let { state.putLong(STATE_FILTER_DATE_TO, it) }
        filter.minimumSizeBytes?.let { state.putLong(STATE_FILTER_MIN_SIZE, it) }
        filter.maximumSizeBytes?.let { state.putLong(STATE_FILTER_MAX_SIZE, it) }
        filter.minimumDurationSeconds?.let { state.putInt(STATE_FILTER_MIN_DURATION, it) }
        filter.maximumDurationSeconds?.let { state.putInt(STATE_FILTER_MAX_DURATION, it) }
        state.putString(STATE_FILTER_EXTENSION, filter.extension)
        state.putString(STATE_FILTER_MIME, filter.mimeType)
        state.putString(STATE_FILTER_THUMBNAIL, filter.thumbnail.name)
        state.putString(STATE_FILTER_RESOLUTION, filter.resolution.name)
    }

    companion object {
        const val EXTRA_SESSION_ID = "mediaLibrary.sessionId"
        const val RESULT_EXIT_REASON = "mediaLibrary.exitReason"
        const val RESULT_ACCOUNT_ID = "mediaLibrary.accountId"
        const val RESULT_ERROR = "mediaLibrary.error"
        private const val STATE_ACCOUNT_ID = "mediaLibrary.safe.accountId"
        private const val STATE_SEARCH = "mediaLibrary.safe.search"
        private const val STATE_SORT = "mediaLibrary.safe.sort"
        private const val STATE_SELECTED_PEER = "mediaLibrary.safe.selectedPeerId"
        private const val STATE_SELECTED_MESSAGE = "mediaLibrary.safe.selectedMessageId"
        private const val STATE_FILTER_SCOPE = "mediaLibrary.safe.filter.scope"
        private const val STATE_FILTER_PEER = "mediaLibrary.safe.filter.peer"
        private const val STATE_FILTER_DATE_FROM = "mediaLibrary.safe.filter.dateFrom"
        private const val STATE_FILTER_DATE_TO = "mediaLibrary.safe.filter.dateTo"
        private const val STATE_FILTER_MIN_SIZE = "mediaLibrary.safe.filter.minSize"
        private const val STATE_FILTER_MAX_SIZE = "mediaLibrary.safe.filter.maxSize"
        private const val STATE_FILTER_MIN_DURATION = "mediaLibrary.safe.filter.minDuration"
        private const val STATE_FILTER_MAX_DURATION = "mediaLibrary.safe.filter.maxDuration"
        private const val STATE_FILTER_EXTENSION = "mediaLibrary.safe.filter.extension"
        private const val STATE_FILTER_MIME = "mediaLibrary.safe.filter.mime"
        private const val STATE_FILTER_THUMBNAIL = "mediaLibrary.safe.filter.thumbnail"
        private const val STATE_FILTER_RESOLUTION = "mediaLibrary.safe.filter.resolution"
    }
}
