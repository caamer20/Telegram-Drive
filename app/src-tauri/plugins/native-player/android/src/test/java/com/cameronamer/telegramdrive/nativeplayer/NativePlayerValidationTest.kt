package com.cameronamer.telegramdrive.nativeplayer

import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class NativePlayerValidationTest {
    private fun validArgs() = OpenNativePlayerArgs().apply {
        folderId = null
        messageId = 9
        title = "Movie"
        fileName = "movie.mkv"
        mimeType = "video/x-matroska"
        streamUrl = "http://127.0.0.1:49152/stream/home/9"
        authorizationToken = "secret"
    }

    @Test
    fun validatesTrustedIdentityRequest() {
        validArgs().validate()
    }

    @Test
    fun rejectsArbitraryUrisAndInvalidIdentity() {
        assertThrows(IllegalArgumentException::class.java) {
            validArgs().apply { streamUrl = "https://example.test/video" }.validate()
        }
        assertThrows(IllegalArgumentException::class.java) {
            validArgs().apply { fileName = "content://media/9" }.validate()
        }
        assertThrows(IllegalArgumentException::class.java) {
            validArgs().apply { messageId = 0 }.validate()
        }
    }

    @Test
    fun publicModelsHaveNoTokenOrUrlProperties() {
        val names = NativePlayerResultData::class.java.declaredFields.map { it.name.lowercase() }
        assertFalse(names.any { it.contains("token") || it.contains("url") })
        val eventFields = NativePlaybackSnapshot::class.java.declaredFields.map { it.name.lowercase() }
        assertFalse(eventFields.any { it.contains("token") || it.contains("authorization") || it.contains("url") })
    }

    @Test
    fun weakRegistryClearsOnlyMatchingInstance() {
        val registry = WeakInstanceRegistry<Any>()
        val first = Any()
        registry.register(first)
        assertTrue(registry.get() === first)
        registry.clear(Any())
        assertTrue(registry.get() === first)
        registry.clear(first)
        assertNull(registry.get())
    }
}
