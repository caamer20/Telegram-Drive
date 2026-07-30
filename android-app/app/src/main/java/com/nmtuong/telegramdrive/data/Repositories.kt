package com.nmtuong.telegramdrive.data

import com.nmtuong.telegramdrive.data.fake.FakeTelegramCatalog
import com.nmtuong.telegramdrive.domain.AuthorizationState
import com.nmtuong.telegramdrive.domain.DataSourceMode
import com.nmtuong.telegramdrive.domain.DiagnosticsState
import com.nmtuong.telegramdrive.telegram.TdLibGateway
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

class RealTelegramRepository(private val gateway: TdLibGateway) : TelegramRepository {
  override val diagnostics: StateFlow<DiagnosticsState> = gateway.state
  override fun start() = gateway.start()
  override fun close() = gateway.close()
}

class FakeTelegramRepository(val catalog: FakeTelegramCatalog) : TelegramRepository {
  private val mutableDiagnostics = MutableStateFlow(
    DiagnosticsState(
      dataSource = DataSourceMode.FAKE,
      authorizationState = AuthorizationState.Other("fakeStableDataset"),
    ),
  )
  override val diagnostics: StateFlow<DiagnosticsState> = mutableDiagnostics
  override fun start() = Unit
  override fun close() = Unit
}
