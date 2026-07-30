package com.nmtuong.telegramdrive.feature.auth

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nmtuong.telegramdrive.R
import com.nmtuong.telegramdrive.domain.*

@Composable
fun AuthorizationScreen(viewModel: AuthorizationViewModel) {
  val session by viewModel.state.collectAsStateWithLifecycle()
  var input by remember(session.state) { mutableStateOf("") }
  Column(Modifier.fillMaxSize().safeDrawingPadding().padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
    Text(stringResource(R.string.auth_title), style = MaterialTheme.typography.headlineMedium)
    when (val state = session.state) {
      AuthorizationState.MissingConfiguration -> Text(stringResource(R.string.missing_configuration))
      AuthorizationState.Unknown, AuthorizationState.WaitingForTdlibParameters -> Text(stringResource(R.string.initializing))
      AuthorizationState.WaitingForPhoneNumber -> {
        AuthInput(R.string.phone_label, input, false, KeyboardType.Phone) { input = it }
        SubmitButton(session.actionPending, input) { viewModel.submit(AuthorizationAction.SubmitPhone(input)); input = "" }
      }
      AuthorizationState.WaitingForCode -> {
        AuthInput(R.string.code_label, input, true, KeyboardType.NumberPassword) { input = it }
        SubmitButton(session.actionPending, input) { viewModel.submit(AuthorizationAction.SubmitCode(input)); input = "" }
      }
      is AuthorizationState.WaitingForPassword -> {
        if (state.hint.isNotBlank()) Text(state.hint)
        AuthInput(R.string.password_label, input, true, KeyboardType.Password) { input = it }
        SubmitButton(session.actionPending, input) { viewModel.submit(AuthorizationAction.SubmitPassword(input)); input = "" }
      }
      AuthorizationState.WaitingForEmailAddress -> {
        AuthInput(R.string.email_label, input, false, KeyboardType.Email) { input = it }
        SubmitButton(session.actionPending, input) { viewModel.submit(AuthorizationAction.SubmitEmailAddress(input)); input = "" }
      }
      AuthorizationState.WaitingForEmailCode -> {
        AuthInput(R.string.email_code_label, input, true, KeyboardType.NumberPassword) { input = it }
        SubmitButton(session.actionPending, input) { viewModel.submit(AuthorizationAction.SubmitEmailCode(input)); input = "" }
      }
      is AuthorizationState.WaitingForOtherDevice -> Text(stringResource(R.string.other_device, state.link))
      AuthorizationState.LoggingOut, AuthorizationState.Closing -> Text(stringResource(R.string.initializing))
      AuthorizationState.Closed -> Text(stringResource(R.string.session_closed))
      AuthorizationState.Ready -> Unit
      is AuthorizationState.Other -> Text(stringResource(R.string.unsupported_state, state.name))
    }
    session.safeError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
  }
}

@Composable private fun AuthInput(label: Int, value: String, secret: Boolean, type: KeyboardType, onValue: (String) -> Unit) {
  OutlinedTextField(
    value = value,
    onValueChange = onValue,
    label = { Text(stringResource(label)) },
    singleLine = true,
    visualTransformation = if (secret) PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
    keyboardOptions = KeyboardOptions(keyboardType = type),
    modifier = Modifier.fillMaxWidth(),
  )
}

@Composable private fun SubmitButton(pending: Boolean, input: String, onClick: () -> Unit) {
  Button(onClick = onClick, enabled = input.isNotBlank() && !pending) { Text(stringResource(R.string.submit)) }
}
