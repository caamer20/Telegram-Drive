package com.nmtuong.telegramdrive.feature.library

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nmtuong.telegramdrive.R
import com.nmtuong.telegramdrive.domain.*

@Composable
fun LibraryScreen(viewModel: LibraryViewModel, onPreview: (PreviewTarget) -> Unit) {
  val state by viewModel.state.collectAsStateWithLifecycle()
  LaunchedEffect(Unit) { viewModel.ensureLoaded() }
  Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
      Text(stringResource(R.string.saved_messages), style = MaterialTheme.typography.headlineSmall)
      TextButton(onClick = viewModel::logout) { Text(stringResource(R.string.logout)) }
    }
    when (val current = state) {
      LibraryState.Idle, LibraryState.Loading -> CircularProgressIndicator()
      LibraryState.Empty -> { Text(stringResource(R.string.empty_library)); Button(onClick = viewModel::refresh) { Text(stringResource(R.string.refresh)) } }
      is LibraryState.Error -> { Text(current.message, color = MaterialTheme.colorScheme.error); Button(onClick = viewModel::refresh) { Text(stringResource(R.string.refresh)) } }
      is LibraryState.Content -> LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        items(current.items, key = { it.fileId }) { item -> MediaRow(item, viewModel, onPreview) }
      }
    }
  }
}

@Composable private fun MediaRow(item: MediaItem, viewModel: LibraryViewModel, onPreview: (PreviewTarget) -> Unit) {
  Card(Modifier.fillMaxWidth()) {
    Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
      Text(item.name, style = MaterialTheme.typography.titleMedium)
      Text(item.kind.name.lowercase())
      if (item.downloadState is DownloadState.Downloading) LinearProgressIndicator(progress = { item.downloadState.percent / 100f })
      if (item.downloadState is DownloadState.Failed) {
        Text(item.downloadState.reason, color = MaterialTheme.colorScheme.error)
      }
      if (item.downloadState == DownloadState.Canceled) Text(stringResource(R.string.download_canceled))
      Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        val target = viewModel.preview(item.id)
        if (target != null) Button(onClick = { onPreview(target) }) { Text(stringResource(R.string.preview)) }
        else if (item.downloadState !is DownloadState.Downloading) {
          Button(onClick = { viewModel.download(item.fileId) }) { Text(stringResource(R.string.download)) }
        }
        if (item.downloadState is DownloadState.Downloading) TextButton(onClick = { viewModel.cancel(item.fileId) }) { Text(stringResource(R.string.cancel)) }
      }
    }
  }
}
