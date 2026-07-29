package com.cameronamer.telegramdrive.nativeplayer

import org.junit.Assert.assertEquals
import org.junit.Test

class CodecCapabilityPreflightTest {
    private fun metadata(codec: String?, bitDepth: Int? = 8) = VideoCapabilityMetadata(
        codec = codec,
        width = 3840,
        height = 2160,
        frameRate = 60.0,
        bitrate = 40_000_000,
        bitDepth = bitDepth,
        hdr = bitDepth == 10,
    )

    @Test
    fun distinguishesHevcMain8AndMain10() {
        val main8Only = listOf(DecoderCapabilityReport("decoder", true, false, true, true))
        assertEquals(
            CapabilityStatus.SUPPORTED,
            CodecCapabilityPreflight.classify(metadata("hvc1"), "hvc1", "video/hevc", main8Only).status,
        )
        assertEquals(
            CapabilityStatus.UNSUPPORTED,
            CodecCapabilityPreflight.classify(metadata("hev1", 10), "hev1", "video/hevc", main8Only).status,
        )
    }

    @Test
    fun rejectsReported4k60LimitAndKeepsUnknownMetadataAdvisory() {
        val limited = listOf(DecoderCapabilityReport("decoder", true, true, false, true))
        assertEquals(
            CapabilityStatus.UNSUPPORTED,
            CodecCapabilityPreflight.classify(metadata("hevc"), "hevc", "video/hevc", limited).status,
        )
        assertEquals(
            CapabilityStatus.UNKNOWN,
            CodecCapabilityPreflight.classify(metadata(null), null, null, limited).status,
        )
    }
}
