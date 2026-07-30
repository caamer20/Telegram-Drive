package com.cameronamer.telegramdrive.medialibrary

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.BrokenImage
import androidx.compose.material.icons.filled.Cancel
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.FilterList
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.NavigateBefore
import androidx.compose.material.icons.filled.NavigateNext
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Sort
import androidx.compose.material.icons.filled.VideoLibrary
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.paging.LoadState
import androidx.paging.compose.collectAsLazyPagingItems
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.cameronamer.telegramdrive.medialibrary.data.MediaFilter
import com.cameronamer.telegramdrive.medialibrary.data.MediaScope
import com.cameronamer.telegramdrive.medialibrary.data.MediaSort
import com.cameronamer.telegramdrive.medialibrary.data.MediaType
import com.cameronamer.telegramdrive.medialibrary.data.ResolutionFilter
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaEntity
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaPeerEntity
import com.cameronamer.telegramdrive.medialibrary.data.ThumbnailFilter
import com.cameronamer.telegramdrive.medialibrary.data.ThumbnailStatus
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

@Composable
fun MediaLibraryTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (androidx.compose.foundation.isSystemInDarkTheme()) darkColorScheme() else lightColorScheme(), content = content)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MediaLibraryScreen(
    viewModel: MediaLibraryViewModel,
    onClose: () -> Unit,
    onPlayVideo: (TelegramMediaEntity) -> Unit,
) {
    val ui by viewModel.uiState.collectAsStateWithLifecycle()
    val progress by viewModel.progress.collectAsStateWithLifecycle()
    val online by viewModel.online.collectAsStateWithLifecycle()
    val items = viewModel.pagingData.collectAsLazyPagingItems()
    val gridState = rememberLazyGridState()
    var showSort by rememberSaveable { mutableStateOf(false) }
    var showFilter by rememberSaveable { mutableStateOf(false) }
    val activeFilter by viewModel.filter.collectAsStateWithLifecycle()
    val activeSort by viewModel.sort.collectAsStateWithLifecycle()
    val search by viewModel.search.collectAsStateWithLifecycle()

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Telegram Media Library") },
                navigationIcon = { IconButton(onClick = onClose) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Close media library") } },
                actions = {
                    IconButton(onClick = { showSort = true }) { Icon(Icons.Default.Sort, "Sort media") }
                    IconButton(onClick = { showFilter = true }) { Icon(Icons.Default.FilterList, "Filter media") }
                    IconButton(enabled = online, onClick = { viewModel.refresh() }) { Icon(Icons.Default.Refresh, "Refresh media") }
                },
            )
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            OutlinedTextField(
                value = search,
                onValueChange = { viewModel.search.value = it },
                modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp),
                singleLine = true,
                label = { Text("Search names, captions, folders, extensions") },
                trailingIcon = { if (search.isNotEmpty()) IconButton(onClick = { viewModel.search.value = "" }) { Icon(Icons.Default.Close, "Clear search") } },
            )
            ActiveFilters(activeFilter, activeSort, ui.peers) { viewModel.setFilter(MediaFilter()) }
            SyncStatus(
                online = online,
                connecting = ui.connecting,
                progress = progress,
                lastSync = ui.lastSuccessfulSync,
                error = ui.error ?: ui.playerError,
                onCancel = viewModel::cancelSync,
                onRetry = viewModel::retrySync,
            )
            when {
                ui.connecting && items.itemCount == 0 -> CenterMessage("Opening local library…", true)
                items.loadState.refresh is LoadState.Loading && items.itemCount == 0 -> CenterMessage("Loading media…", true)
                items.loadState.refresh is LoadState.Error && items.itemCount == 0 -> CenterMessage("Local library could not be read", false)
                items.itemCount == 0 -> CenterMessage(if (online) "No synchronized media matches this view" else "No offline media is available for this account", false)
                else -> {
                    LazyVerticalGrid(
                        columns = GridCells.Adaptive(128.dp),
                        state = gridState,
                        modifier = Modifier.weight(1f).fillMaxWidth(),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        contentPadding = androidx.compose.foundation.layout.PaddingValues(8.dp),
                    ) {
                        items(items.itemCount, key = { index -> items[index]?.stableKey ?: "placeholder_$index" }) { index ->
                            items[index]?.let { item ->
                                MediaGridCard(item, viewModel, onClick = { viewModel.select(item) })
                            }
                        }
                        when (items.loadState.append) {
                            is LoadState.Loading -> item { Box(Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) { CircularProgressIndicator() } }
                            is LoadState.Error -> item { TextButton(onClick = items::retry) { Text("Retry loading more") } }
                            else -> Unit
                        }
                    }
                }
            }
        }
    }

    if (showSort) SortDialog(activeSort, { showSort = false }) { viewModel.setSort(it); showSort = false }
    if (showFilter) FilterDialog(activeFilter, ui.peers, { showFilter = false }) { viewModel.setFilter(it); showFilter = false }
    ui.selected?.let { selected ->
        val snapshot = items.itemSnapshotList.items
        val index = snapshot.indexOfFirst { it.stableKey == selected.stableKey }
        MediaPreview(
            item = selected,
            online = online,
            viewModel = viewModel,
            onClose = { viewModel.select(null) },
            onPrevious = snapshot.getOrNull(index - 1)?.let { previous -> { viewModel.select(previous) } },
            onNext = snapshot.getOrNull(index + 1)?.let { next -> { viewModel.select(next) } },
            onPlay = { onPlayVideo(selected) },
        )
    }
}

@Composable
private fun MediaGridCard(item: TelegramMediaEntity, viewModel: MediaLibraryViewModel, onClick: () -> Unit) {
    var localFile by remember(item.stableKey, item.thumbnailPath) { mutableStateOf(item.thumbnailPath?.let(::File)?.takeIf(File::exists)) }
    var thumbnailLoading by remember(item.stableKey, item.thumbnailPath) {
        mutableStateOf(localFile == null && item.thumbnailAvailable && item.thumbnailStatus != ThumbnailStatus.FAILED)
    }
    val thumbnailScope = rememberCoroutineScope()
    LaunchedEffect(item.stableKey, item.thumbnailPath) {
        if (localFile == null) {
            thumbnailLoading = item.thumbnailAvailable && item.thumbnailStatus != ThumbnailStatus.FAILED
            localFile = withContext(Dispatchers.IO) { viewModel.ensureThumbnail(item) }
            thumbnailLoading = false
        }
    }
    DisposableEffect(localFile?.absolutePath) {
        val retainedFile = localFile
        retainedFile?.let(viewModel::retainThumbnail)
        onDispose { retainedFile?.let(viewModel::releaseThumbnail) }
    }
    Surface(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).semantics { contentDescription = "Open ${item.displayName}" },
        tonalElevation = 2.dp,
        shape = RoundedCornerShape(10.dp),
    ) {
        Column {
            Box(Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(topStart = 10.dp, topEnd = 10.dp)).background(MaterialTheme.colorScheme.surfaceVariant)) {
                if (localFile != null) {
                    AsyncImage(
                        model = ImageRequest.Builder(LocalContext.current).data(localFile).crossfade(true).build(),
                        contentDescription = "Thumbnail for ${item.displayName}",
                        contentScale = ContentScale.Crop,
                        modifier = Modifier.fillMaxSize(),
                        onError = { localFile = null },
                    )
                } else if (thumbnailLoading) {
                    CircularProgressIndicator(Modifier.align(Alignment.Center).size(34.dp))
                } else if (item.thumbnailAvailable) {
                    Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) {
                        Icon(Icons.Default.BrokenImage, "Thumbnail failed", Modifier.size(32.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        TextButton(onClick = {
                            thumbnailLoading = true
                            thumbnailScope.launch {
                                localFile = withContext(Dispatchers.IO) { viewModel.ensureThumbnail(item, retry = true) }
                                thumbnailLoading = false
                            }
                        }) { Text("Retry") }
                    }
                } else {
                    Icon(Icons.Default.BrokenImage, "Thumbnail unavailable", Modifier.align(Alignment.Center).size(38.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Icon(
                    if (item.mediaType == MediaType.VIDEO) Icons.Default.VideoLibrary else Icons.Default.Image,
                    if (item.mediaType == MediaType.VIDEO) "Video" else "Image",
                    Modifier.align(Alignment.TopEnd).padding(6.dp).background(Color.Black.copy(alpha = .55f), RoundedCornerShape(4.dp)).padding(3.dp),
                    tint = Color.White,
                )
                item.durationSeconds?.let { duration ->
                    Text(formatDuration(duration), color = Color.White, style = MaterialTheme.typography.labelSmall,
                        modifier = Modifier.align(Alignment.BottomEnd).padding(5.dp).background(Color.Black.copy(alpha = .65f), RoundedCornerShape(4.dp)).padding(horizontal = 4.dp, vertical = 2.dp))
                }
            }
            Text(item.displayName, maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(8.dp, 7.dp, 8.dp, 2.dp))
            Text(listOfNotNull(formatDate(item.dateEpochSeconds), item.sizeBytes?.let(::formatBytes)).joinToString(" • "),
                maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(8.dp, 0.dp, 8.dp, 7.dp))
        }
    }
}

@Composable
private fun SyncStatus(
    online: Boolean,
    connecting: Boolean,
    progress: com.cameronamer.telegramdrive.medialibrary.repository.SyncProgress,
    lastSync: Long?,
    error: String?,
    onCancel: () -> Unit,
    onRetry: () -> Unit,
) {
    val running = progress.fullSyncRunning || progress.incrementalRefreshRunning || progress.cancellationAvailable
    Column(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp)) {
        if (!online && !connecting) Text("Offline — reconnect the main Telegram runtime to sync or play video", color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
        if (running) {
            LinearProgressIndicator(
                progress = { if (progress.totalPeers == 0) 0f else progress.completedPeers.toFloat() / progress.totalPeers },
                modifier = Modifier.fillMaxWidth(),
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("${if (progress.fullSyncRunning) "Full sync" else "Refreshing"}: ${progress.currentPeerName ?: "peers"} • ${progress.mediaIndexed} media / ${progress.messagesScanned} messages",
                    style = MaterialTheme.typography.labelSmall, modifier = Modifier.weight(1f))
                if (progress.cancellationAvailable) IconButton(onClick = onCancel) { Icon(Icons.Default.Cancel, "Cancel synchronization") }
            }
        } else {
            val label = lastSync?.let { "Last synchronized ${formatDateTime(it)}" } ?: "Not synchronized yet"
            Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        (progress.lastError ?: error)?.let { message ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(message, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = Modifier.weight(1f))
                if (online) TextButton(onClick = onRetry) { Text("Retry") }
            }
        }
    }
}

@Composable
private fun ActiveFilters(filter: MediaFilter, sort: MediaSort, peers: List<TelegramMediaPeerEntity>, clear: () -> Unit) {
    val labels = buildList {
        if (filter.scope != MediaScope.ALL) add(filter.scope.name.lowercase().replaceFirstChar(Char::uppercase))
        filter.peerId?.let { id -> add(peers.firstOrNull { it.peerId == id }?.name ?: "Peer $id") }
        filter.extension?.takeIf(String::isNotBlank)?.let { add(".$it") }
        filter.mimeType?.takeIf(String::isNotBlank)?.let(::add)
        if (filter.thumbnail != ThumbnailFilter.ANY) add(filter.thumbnail.name.lowercase().replace('_', ' '))
        if (filter.resolution != ResolutionFilter.ANY) add(filter.resolution.name.replace('_', ' '))
    }
    if (labels.isEmpty() && sort == MediaSort.NEWEST) return
    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp).horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        labels.take(4).forEach { AssistChip(onClick = {}, label = { Text(it) }) }
        AssistChip(onClick = {}, label = { Text(sort.name.lowercase().replace('_', ' ')) })
        if (labels.isNotEmpty()) AssistChip(onClick = clear, label = { Text("Clear filters") })
    }
}

@Composable
private fun CenterMessage(message: String, loading: Boolean) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (loading) CircularProgressIndicator()
            Text(message, modifier = Modifier.padding(24.dp))
        }
    }
}

@Composable
private fun SortDialog(current: MediaSort, dismiss: () -> Unit, select: (MediaSort) -> Unit) {
    AlertDialog(
        onDismissRequest = dismiss,
        title = { Text("Sort media") },
        text = { Column(Modifier.verticalScroll(rememberScrollState())) { MediaSort.entries.forEach { value ->
            FilterChip(selected = current == value, onClick = { select(value) }, label = { Text(sortLabel(value)) }, modifier = Modifier.fillMaxWidth())
        } } },
        confirmButton = { TextButton(onClick = dismiss) { Text("Close") } },
    )
}

@Composable
private fun FilterDialog(current: MediaFilter, peers: List<TelegramMediaPeerEntity>, dismiss: () -> Unit, apply: (MediaFilter) -> Unit) {
    var value by remember(current) { mutableStateOf(current) }
    var minSize by remember(current) { mutableStateOf(current.minimumSizeBytes?.div(1024L * 1024L)?.toString().orEmpty()) }
    var maxSize by remember(current) { mutableStateOf(current.maximumSizeBytes?.div(1024L * 1024L)?.toString().orEmpty()) }
    var minDuration by remember(current) { mutableStateOf(current.minimumDurationSeconds?.toString().orEmpty()) }
    var maxDuration by remember(current) { mutableStateOf(current.maximumDurationSeconds?.toString().orEmpty()) }
    var dateFrom by remember(current) { mutableStateOf(current.dateFromEpochSeconds?.toString().orEmpty()) }
    var dateTo by remember(current) { mutableStateOf(current.dateToEpochSeconds?.toString().orEmpty()) }
    AlertDialog(
        onDismissRequest = dismiss,
        title = { Text("Filter media") },
        text = {
            Column(Modifier.fillMaxHeight(.72f).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Media type", style = MaterialTheme.typography.titleSmall)
                Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) { MediaScope.entries.forEach { scope -> FilterChip(value.scope == scope, { value = value.copy(scope = scope) }, { Text(scope.name.lowercase()) }) } }
                Text("Folder", style = MaterialTheme.typography.titleSmall)
                FilterChip(value.peerId == null, { value = value.copy(peerId = null) }, { Text("All folders") })
                peers.forEach { peer -> FilterChip(value.peerId == peer.peerId, { value = value.copy(peerId = peer.peerId) }, { Text(peer.name) }, modifier = Modifier.fillMaxWidth()) }
                Text("Thumbnail", style = MaterialTheme.typography.titleSmall)
                Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) { ThumbnailFilter.entries.forEach { option -> FilterChip(value.thumbnail == option, { value = value.copy(thumbnail = option) }, { Text(option.name.lowercase().replace('_', ' ')) }) } }
                Text("Resolution", style = MaterialTheme.typography.titleSmall)
                ResolutionFilter.entries.forEach { option -> FilterChip(value.resolution == option, { value = value.copy(resolution = option) }, { Text(option.name.replace('_', ' ')) }) }
                OutlinedTextField(value.extension.orEmpty(), { value = value.copy(extension = it) }, label = { Text("Extension") }, singleLine = true)
                OutlinedTextField(value.mimeType.orEmpty(), { value = value.copy(mimeType = it) }, label = { Text("MIME type") }, singleLine = true)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(minSize, { minSize = it }, Modifier.weight(1f), label = { Text("Min MB") }, singleLine = true)
                    OutlinedTextField(maxSize, { maxSize = it }, Modifier.weight(1f), label = { Text("Max MB") }, singleLine = true)
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(minDuration, { minDuration = it }, Modifier.weight(1f), label = { Text("Min seconds") }, singleLine = true)
                    OutlinedTextField(maxDuration, { maxDuration = it }, Modifier.weight(1f), label = { Text("Max seconds") }, singleLine = true)
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(dateFrom, { dateFrom = it }, Modifier.weight(1f), label = { Text("From epoch") }, singleLine = true)
                    OutlinedTextField(dateTo, { dateTo = it }, Modifier.weight(1f), label = { Text("To epoch") }, singleLine = true)
                }
            }
        },
        confirmButton = { Button(onClick = {
            apply(value.copy(
                minimumSizeBytes = minSize.toLongOrNull()?.times(1024L * 1024L),
                maximumSizeBytes = maxSize.toLongOrNull()?.times(1024L * 1024L),
                minimumDurationSeconds = minDuration.toIntOrNull(),
                maximumDurationSeconds = maxDuration.toIntOrNull(),
                dateFromEpochSeconds = dateFrom.toLongOrNull(),
                dateToEpochSeconds = dateTo.toLongOrNull(),
            ))
        }) { Text("Apply") } },
        dismissButton = { TextButton(onClick = { apply(MediaFilter()) }) { Text("Reset") } },
    )
}

@Composable
private fun MediaPreview(
    item: TelegramMediaEntity,
    online: Boolean,
    viewModel: MediaLibraryViewModel,
    onClose: () -> Unit,
    onPrevious: (() -> Unit)?,
    onNext: (() -> Unit)?,
    onPlay: () -> Unit,
) {
    var localFile by remember(item.stableKey, item.thumbnailPath) { mutableStateOf(item.thumbnailPath?.let(::File)?.takeIf(File::exists)) }
    var thumbnailLoading by remember(item.stableKey, item.thumbnailPath) {
        mutableStateOf(localFile == null && item.thumbnailAvailable && item.thumbnailStatus != ThumbnailStatus.FAILED)
    }
    var details by rememberSaveable(item.stableKey) { mutableStateOf(false) }
    val thumbnailScope = rememberCoroutineScope()
    LaunchedEffect(item.stableKey) {
        if (localFile == null) {
            thumbnailLoading = item.thumbnailAvailable && item.thumbnailStatus != ThumbnailStatus.FAILED
            localFile = withContext(Dispatchers.IO) { viewModel.ensureThumbnail(item, 1024) } ?: localFile
            thumbnailLoading = false
        }
    }
    DisposableEffect(localFile?.absolutePath) {
        val retainedFile = localFile
        retainedFile?.let(viewModel::retainThumbnail)
        onDispose { retainedFile?.let(viewModel::releaseThumbnail) }
    }
    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        Surface(Modifier.fillMaxSize(), color = Color.Black) {
            Box(Modifier.fillMaxSize()) {
                if (localFile != null) AsyncImage(localFile, "Preview of ${item.displayName}", Modifier.fillMaxSize(), contentScale = ContentScale.Fit)
                else if (thumbnailLoading) CircularProgressIndicator(Modifier.align(Alignment.Center), color = Color.White)
                else Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) {
                    Icon(Icons.Default.BrokenImage, "Preview unavailable", Modifier.size(64.dp), tint = Color.White)
                    if (item.thumbnailAvailable) TextButton(onClick = {
                        thumbnailLoading = true
                        thumbnailScope.launch {
                            localFile = withContext(Dispatchers.IO) { viewModel.ensureThumbnail(item, 1024, retry = true) }
                            thumbnailLoading = false
                        }
                    }) { Text("Retry preview") }
                }
                Row(Modifier.align(Alignment.TopCenter).fillMaxWidth().background(Color.Black.copy(alpha = .5f)).padding(5.dp), verticalAlignment = Alignment.CenterVertically) {
                    IconButton(onClick = onClose) { Icon(Icons.Default.Close, "Close preview", tint = Color.White) }
                    Text(item.displayName, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                    IconButton(onClick = { details = true }) { Icon(Icons.Default.Info, "Media details", tint = Color.White) }
                }
                Row(Modifier.align(Alignment.Center).fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    IconButton(enabled = onPrevious != null, onClick = { onPrevious?.invoke() }) { Icon(Icons.Default.NavigateBefore, "Previous media", tint = Color.White, modifier = Modifier.size(48.dp)) }
                    IconButton(enabled = onNext != null, onClick = { onNext?.invoke() }) { Icon(Icons.Default.NavigateNext, "Next media", tint = Color.White, modifier = Modifier.size(48.dp)) }
                }
                if (item.mediaType == MediaType.VIDEO) {
                    Button(enabled = online, onClick = onPlay, modifier = Modifier.align(Alignment.BottomCenter).padding(32.dp)) {
                        Icon(Icons.Default.PlayArrow, null); Spacer(Modifier.width(6.dp)); Text(if (online) "Play in native player" else "Reconnect to play")
                    }
                }
            }
        }
    }
    if (details) DetailsDialog(item) { details = false }
}

@Composable
private fun DetailsDialog(item: TelegramMediaEntity, dismiss: () -> Unit) {
    val rows = listOf(
        "Display name" to item.displayName,
        "Original filename" to item.originalFilename,
        "Caption" to item.caption,
        "Folder" to item.peerName,
        "MIME type" to item.mimeType,
        "Extension" to item.extension,
        "Size" to item.sizeBytes?.let(::formatBytes),
        "Date" to formatDateTime(item.dateEpochSeconds),
        "Dimensions" to if (item.width != null && item.height != null) "${item.width} × ${item.height}" else null,
        "Duration" to item.durationSeconds?.let(::formatDuration),
        "Media type" to item.mediaType.name.lowercase().replace('_', ' '),
    )
    AlertDialog(onDismissRequest = dismiss, title = { Text("Media details") }, text = {
        Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            rows.forEach { (label, raw) -> raw?.takeIf(String::isNotBlank)?.let { Text("$label\n$it", style = MaterialTheme.typography.bodyMedium) } }
        }
    }, confirmButton = { TextButton(onClick = dismiss) { Text("Close") } })
}

private fun sortLabel(sort: MediaSort): String = when (sort) {
    MediaSort.NEWEST -> "Newest first"
    MediaSort.OLDEST -> "Oldest first"
    MediaSort.NAME_ASC -> "Name A–Z"
    MediaSort.NAME_DESC -> "Name Z–A"
    MediaSort.LARGEST -> "Largest first"
    MediaSort.SMALLEST -> "Smallest first"
    MediaSort.LONGEST_VIDEO -> "Longest video first"
    MediaSort.SHORTEST_VIDEO -> "Shortest video first"
    MediaSort.FOLDER_ASC -> "Folder A–Z"
    MediaSort.FOLDER_DESC -> "Folder Z–A"
}

private fun formatDuration(seconds: Int): String = "%d:%02d".format(seconds / 60, seconds % 60)
private fun formatBytes(bytes: Long): String = when {
    bytes >= 1_073_741_824 -> "%.1f GB".format(bytes / 1_073_741_824.0)
    bytes >= 1_048_576 -> "%.1f MB".format(bytes / 1_048_576.0)
    bytes >= 1024 -> "%.1f KB".format(bytes / 1024.0)
    else -> "$bytes B"
}
private fun formatDate(epoch: Long): String = SimpleDateFormat("yyyy-MM-dd", Locale.US).format(Date(epoch * 1_000))
private fun formatDateTime(epoch: Long): String = SimpleDateFormat("yyyy-MM-dd HH:mm", Locale.US).format(Date(epoch * 1_000))
