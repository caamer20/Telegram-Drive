package com.nmtuong.telegramdrive.telegram

import com.nmtuong.telegramdrive.domain.DiagnosticsState
import java.io.Closeable
import kotlinx.coroutines.flow.StateFlow

interface TdLibGateway : Closeable {
  val state: StateFlow<DiagnosticsState>
  fun start()
}
