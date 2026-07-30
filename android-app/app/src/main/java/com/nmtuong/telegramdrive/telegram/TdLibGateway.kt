package com.nmtuong.telegramdrive.telegram

import com.nmtuong.telegramdrive.domain.*
import java.io.Closeable
import kotlinx.coroutines.flow.StateFlow

interface TdLibGateway : Closeable {
  val state: StateFlow<DiagnosticsState>
  val authorization: StateFlow<AuthorizationSession>
  val library: StateFlow<LibraryState>
  fun start()
  fun submit(action: AuthorizationAction): ActionResult
  fun loadSavedMessages(limit: Int): ActionResult
  fun download(fileId: Int): ActionResult
  fun cancelDownload(fileId: Int): ActionResult
  fun preview(itemId: Long): PreviewTarget?
}
