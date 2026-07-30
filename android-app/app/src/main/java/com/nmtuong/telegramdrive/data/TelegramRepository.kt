package com.nmtuong.telegramdrive.data

import com.nmtuong.telegramdrive.domain.DiagnosticsState
import java.io.Closeable
import kotlinx.coroutines.flow.StateFlow

interface TelegramRepository : Closeable {
  val diagnostics: StateFlow<DiagnosticsState>
  fun start()
}
