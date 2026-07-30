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
}
