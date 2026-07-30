package com.nmtuong.telegramdrive.feature.diagnostics

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle

@Composable
fun DiagnosticsScreen(viewModel: DiagnosticsViewModel) {
  val state by viewModel.state.collectAsStateWithLifecycle()
  Column(
    Modifier.fillMaxSize().verticalScroll(rememberScrollState()).safeDrawingPadding().padding(20.dp)
      .semantics { testTag = "phase-zero-diagnostics" },
    verticalArrangement = Arrangement.spacedBy(12.dp),
  ) {
    Text("Telegram Drive", style = MaterialTheme.typography.headlineMedium)
    Text("Phase 0 diagnostics", style = MaterialTheme.typography.titleMedium)
    DiagnosticRow("Data source", state.dataSource.id)
    DiagnosticRow("Lifecycle", state.lifecycle.name.lowercase())
    DiagnosticRow("Native library", if (state.nativeLibraryLoaded) "loaded" else "not loaded")
    DiagnosticRow("TDLib client", if (state.clientCreated) "created" else "not created")
    DiagnosticRow("Authorization", state.authorizationState.javaClass.simpleName)
    DiagnosticRow("Active clients", state.clientInstanceCount.toString())
    state.safeError?.let { Text("Safe error: $it", color = MaterialTheme.colorScheme.error) }
    HorizontalDivider()
    Text("Fake dataset ready", style = MaterialTheme.typography.titleMedium)
    DiagnosticRow("Account", viewModel.sampleCatalog.account.displayName)
    DiagnosticRow("Sources", viewModel.sampleCatalog.sources.size.toString())
    DiagnosticRow("Media samples", viewModel.sampleCatalog.media.size.toString())
  }
}

@Composable private fun DiagnosticRow(label: String, value: String) {
  Card(Modifier.fillMaxWidth()) {
    Column(Modifier.padding(14.dp)) {
      Text(label, style = MaterialTheme.typography.labelLarge)
      Text(value, style = MaterialTheme.typography.bodyLarge)
    }
  }
}
