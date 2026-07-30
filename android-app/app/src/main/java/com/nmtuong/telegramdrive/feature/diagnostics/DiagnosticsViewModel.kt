package com.nmtuong.telegramdrive.feature.diagnostics

import androidx.lifecycle.ViewModel
import com.nmtuong.telegramdrive.data.TelegramRepository
import com.nmtuong.telegramdrive.data.fake.FakeTelegramCatalog

class DiagnosticsViewModel(repository: TelegramRepository, val sampleCatalog: FakeTelegramCatalog) : ViewModel() {
  val state = repository.diagnostics
}
