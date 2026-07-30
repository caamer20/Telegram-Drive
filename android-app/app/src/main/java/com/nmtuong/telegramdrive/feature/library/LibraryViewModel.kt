package com.nmtuong.telegramdrive.feature.library

import androidx.lifecycle.ViewModel
import com.nmtuong.telegramdrive.data.TelegramRepository
import com.nmtuong.telegramdrive.domain.*

class LibraryViewModel(private val repository: TelegramRepository) : ViewModel() {
  val state = repository.library
  fun ensureLoaded() { if (state.value == LibraryState.Idle) repository.loadSavedMessages() }
  fun refresh() = repository.loadSavedMessages()
  fun logout() = repository.submit(AuthorizationAction.Logout)
  fun download(fileId: Int) = repository.download(fileId)
  fun cancel(fileId: Int) = repository.cancelDownload(fileId)
  fun preview(itemId: Long) = repository.preview(itemId)
}
