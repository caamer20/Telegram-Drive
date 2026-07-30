package com.nmtuong.telegramdrive.navigation

import androidx.compose.runtime.Composable
import androidx.lifecycle.viewmodel.compose.viewModel
import com.nmtuong.telegramdrive.bootstrap.AppContainer
import com.nmtuong.telegramdrive.feature.diagnostics.DiagnosticsScreen
import com.nmtuong.telegramdrive.feature.diagnostics.DiagnosticsViewModel

/** Phase 0 navigation boundary; later features can add destinations without changing Activity. */
@Composable
fun AppNavigation(container: AppContainer) {
  val diagnosticsViewModel: DiagnosticsViewModel = viewModel {
    DiagnosticsViewModel(container.telegramRepository, container.sampleCatalog)
  }
  DiagnosticsScreen(diagnosticsViewModel)
}
