package com.nmtuong.telegramdrive

import com.nmtuong.telegramdrive.data.RealTelegramRepository
import com.nmtuong.telegramdrive.domain.DataSourceMode
import com.nmtuong.telegramdrive.domain.DiagnosticsState
import com.nmtuong.telegramdrive.domain.*
import com.nmtuong.telegramdrive.telegram.TdLibGateway
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.junit.Assert.assertEquals
import org.junit.Test

class RepositoryBoundaryTest {
  @Test fun gatewayCanBeStartedClosedAndReplacedWithoutUiDependency() {
    val gateway = RecordingGateway()
    val repository = RealTelegramRepository(gateway)
    repository.start()
    repository.close()
    assertEquals(1, gateway.starts)
    assertEquals(1, gateway.closes)
    assertEquals(DataSourceMode.REAL, repository.diagnostics.value.dataSource)
  }
}

private class RecordingGateway : TdLibGateway {
  override val state: StateFlow<DiagnosticsState> =
    MutableStateFlow(DiagnosticsState(dataSource = DataSourceMode.REAL))
  var starts = 0
  var closes = 0
  override val authorization: StateFlow<AuthorizationSession> = MutableStateFlow(AuthorizationSession())
  override val library: StateFlow<LibraryState> = MutableStateFlow(LibraryState.Idle)
  override fun start() { starts++ }
  override fun submit(action: AuthorizationAction) = ActionResult.ACCEPTED
  override fun loadSavedMessages(limit: Int) = ActionResult.ACCEPTED
  override fun download(fileId: Int) = ActionResult.ACCEPTED
  override fun cancelDownload(fileId: Int) = ActionResult.ACCEPTED
  override fun preview(itemId: Long): PreviewTarget? = null
  override fun close() { closes++ }
}
