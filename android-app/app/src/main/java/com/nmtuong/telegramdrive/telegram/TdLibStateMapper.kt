package com.nmtuong.telegramdrive.telegram

import com.nmtuong.telegramdrive.domain.AuthorizationState
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.contentOrNull

object TdLibStateMapper {
  fun authorizationState(json: String): AuthorizationState? {
    val root = runCatching { Json.parseToJsonElement(json).jsonObject }.getOrNull() ?: return null
    val state = if (root["@type"]?.jsonPrimitive?.contentOrNull == "updateAuthorizationState") {
      root["authorization_state"]?.jsonObject ?: return null
    } else root
    val name = state["@type"]?.jsonPrimitive?.contentOrNull?.takeIf { it.startsWith("authorizationState") } ?: return null
    return when (name) {
      "authorizationStateWaitTdlibParameters" -> AuthorizationState.WaitingForTdlibParameters
      "authorizationStateWaitPhoneNumber" -> AuthorizationState.WaitingForPhoneNumber
      "authorizationStateWaitCode" -> AuthorizationState.WaitingForCode
      "authorizationStateWaitPassword" -> AuthorizationState.WaitingForPassword(state["password_hint"]?.jsonPrimitive?.contentOrNull.orEmpty())
      "authorizationStateWaitEmailAddress" -> AuthorizationState.WaitingForEmailAddress
      "authorizationStateWaitEmailCode" -> AuthorizationState.WaitingForEmailCode
      "authorizationStateWaitOtherDeviceConfirmation" -> AuthorizationState.WaitingForOtherDevice(state["link"]?.jsonPrimitive?.contentOrNull.orEmpty())
      "authorizationStateReady" -> AuthorizationState.Ready
      "authorizationStateLoggingOut" -> AuthorizationState.LoggingOut
      "authorizationStateClosing" -> AuthorizationState.Closing
      "authorizationStateClosed" -> AuthorizationState.Closed
      else -> AuthorizationState.Other(name)
    }
  }
}
