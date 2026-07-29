package com.cameronamer.telegramdrive.nativeplayer

import org.junit.Assert.assertEquals
import org.junit.Test

class NativePlayerErrorMapperTest {
    @Test
    fun mapsAuthenticationAndRangeStatuses() {
        assertEquals("authentication", NativePlayerErrorMapper.mapHttpStatus(401).category)
        assertEquals("authentication", NativePlayerErrorMapper.mapHttpStatus(403).category)
        assertEquals("server", NativePlayerErrorMapper.mapHttpStatus(416).category)
    }

    @Test
    fun mapsNotFoundAndServerStatuses() {
        assertEquals("server", NativePlayerErrorMapper.mapHttpStatus(404).category)
        assertEquals("server", NativePlayerErrorMapper.mapHttpStatus(503).category)
    }
}
