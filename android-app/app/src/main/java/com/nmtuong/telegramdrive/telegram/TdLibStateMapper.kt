package com.nmtuong.telegramdrive.telegram

import com.nmtuong.telegramdrive.domain.AuthorizationState

object TdLibStateMapper {
  private val typeRegex = Regex("\\\"@type\\\"\\s*:\\s*\\\"([^\\\"]+)\\\"")

  fun authorizationState(json: String): AuthorizationState? {
    val name = typeRegex.findAll(json)
      .map { it.groupValues[1] }
      .firstOrNull { it.startsWith("authorizationState") } ?: return null
    return when (name) {
      "authorizationStateWaitTdlibParameters" -> AuthorizationState.WaitingForTdlibParameters
      "authorizationStateWaitPhoneNumber" -> AuthorizationState.WaitingForPhoneNumber
      "authorizationStateReady" -> AuthorizationState.Ready
      "authorizationStateClosed" -> AuthorizationState.Closed
      else -> AuthorizationState.Other(name)
    }
  }
}
