package com.nmtuong.telegramdrive

import com.nmtuong.telegramdrive.domain.AuthorizationState
import com.nmtuong.telegramdrive.telegram.TdLibStateMapper
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TdLibStateMapperTest {
  @Test fun mapsFirstAuthorizationStateWithoutGeneratedModels() {
    val json = "{\"@type\":\"authorizationStateWaitTdlibParameters\",\"@extra\":\"phase-0\"}"
    assertEquals(AuthorizationState.WaitingForTdlibParameters, TdLibStateMapper.authorizationState(json))
  }
  @Test fun ignoresUnrelatedUpdates() = assertNull(TdLibStateMapper.authorizationState("{\"@type\":\"ok\"}"))
  @Test fun mapsNestedAuthorizationStateDetails() {
    val password = """{"@type":"updateAuthorizationState","authorization_state":{"@type":"authorizationStateWaitPassword","password_hint":"pet"}}"""
    val otherDevice = """{"@type":"authorizationStateWaitOtherDeviceConfirmation","link":"tg://login?token=safe-placeholder"}"""
    assertEquals(AuthorizationState.WaitingForPassword("pet"), TdLibStateMapper.authorizationState(password))
    assertEquals(AuthorizationState.WaitingForOtherDevice("tg://login?token=safe-placeholder"), TdLibStateMapper.authorizationState(otherDevice))
  }
}
