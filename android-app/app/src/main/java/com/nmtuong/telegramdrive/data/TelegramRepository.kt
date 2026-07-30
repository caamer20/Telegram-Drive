package com.nmtuong.telegramdrive.data

import com.nmtuong.telegramdrive.domain.*
import java.io.Closeable
import kotlinx.coroutines.flow.StateFlow

interface TelegramRepository : Closeable {
  val diagnostics: StateFlow<DiagnosticsState>
  val authorization: StateFlow<AuthorizationSession>
  val library: StateFlow<LibraryState>
  fun start()
  fun submit(action: AuthorizationAction): ActionResult
  fun loadSavedMessages(limit: Int = 50): ActionResult
  fun download(fileId: Int): ActionResult
  fun cancelDownload(fileId: Int): ActionResult
  fun preview(itemId: Long): PreviewTarget?
}
