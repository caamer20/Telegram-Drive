package com.cameronamer.telegramdrive.nativeplayer

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

class CodecCapabilityPreflightTest {
    private fun metadata(
        codecs: String?,
        width: Int? = 3840,
        height: Int? = 2160,
        frameRate: Double? = 60.0,
        bitrate: Long? = 40_000_000,
        hdr: HdrType = HdrType.SDR,
    ) = VideoCapabilityMetadata(
        sampleMimeType = "video/hevc",
        codecs = codecs,
        width = width,
        height = height,
        frameRate = frameRate,
        averageBitrate = bitrate,
        hdrType = hdr,
    )

    private fun decoder(
        profile: Boolean? = true,
        level: Boolean? = true,
        size: Boolean? = true,
        rate: Boolean? = true,
        bitrate: Boolean? = true,
        hardware: Boolean = true,
    ) = DecoderCapabilityReport(
        "decoder",
        hardware,
        !hardware,
        hardware,
        false,
        profile,
        level,
        size,
        rate,
        bitrate,
    )

    @Test
    fun distinguishesHevcMain8Main10AndKeepsHev1Tag() {
        val main = CodecCapabilityPreflight.classify(metadata("hvc1.1.6.L120"), "video/hevc", listOf(decoder()))
        val main10 = CodecCapabilityPreflight.classify(metadata("hev1.2.4.L153"), "video/hevc", listOf(decoder()))
        assertEquals(HevcProfile.MAIN_8, main.hevcProfile)
        assertEquals(HevcProfile.MAIN_10, main10.hevcProfile)
        assertEquals("hev1.2.4.L153", main10.codecTag)
        assertEquals(CapabilityStatus.SUPPORTED, main10.status)
    }

    @Test
    fun genericHevcDecoderNeverImpliesMain10() {
        val result = CodecCapabilityPreflight.classify(
            metadata("video/hevc"),
            "video/hevc",
            listOf(decoder(profile = null, level = null)),
        )
        assertEquals(HevcProfile.UNKNOWN, result.hevcProfile)
        assertEquals(CapabilityStatus.LIKELY_SUPPORTED, result.status)
        assertNotEquals("UNSUPPORTED_VIDEO_PROFILE", result.reasonCode)
    }

    @Test
    fun classifiesUnsupportedProfileResolutionRateAndBitrate() {
        assertEquals(
            "UNSUPPORTED_PROFILE",
            CodecCapabilityPreflight.classify(metadata("hvc1.2.4.L153"), "video/hevc", listOf(decoder(profile = false))).reasonCode,
        )
        assertEquals(
            "UNSUPPORTED_RESOLUTION",
            CodecCapabilityPreflight.classify(metadata("hvc1.1.6.L120"), "video/hevc", listOf(decoder(size = false))).reasonCode,
        )
        assertEquals(
            "UNSUPPORTED_FRAME_RATE",
            CodecCapabilityPreflight.classify(metadata("hvc1.1.6.L120"), "video/hevc", listOf(decoder(rate = false))).reasonCode,
        )
        assertEquals(
            "UNSUPPORTED_BITRATE",
            CodecCapabilityPreflight.classify(metadata("hvc1.1.6.L120"), "video/hevc", listOf(decoder(bitrate = false))).reasonCode,
        )
    }

    @Test
    fun unknownDimensionsAndRatesStayAdvisory() {
        val result = CodecCapabilityPreflight.classify(
            metadata("hvc1.1.6.L120", width = null, height = null, frameRate = null, bitrate = null),
            "video/hevc",
            listOf(decoder(size = null, rate = null, bitrate = null)),
        )
        assertEquals(CapabilityStatus.LIKELY_SUPPORTED, result.status)
        assertEquals(CapabilityStatus.UNKNOWN, result.resolutionStatus)
        assertEquals(CapabilityStatus.UNKNOWN, result.frameRateStatus)
        assertEquals(CapabilityStatus.UNKNOWN, result.bitrateStatus)
    }

    @Test
    fun detectsHdrUncertaintyDolbyVisionAndExplicitUnsupportedChroma() {
        val hdr = CodecCapabilityPreflight.classify(
            metadata("hvc1.2.4.L153", hdr = HdrType.HDR10),
            "video/hevc",
            listOf(decoder()),
        )
        assertEquals(CapabilityStatus.LIKELY_SUPPORTED, hdr.status)
        assertEquals(true, hdr.displayVerificationRequired)
        assertEquals(
            "video/dolby-vision",
            CodecCapabilityPreflight.resolveMimeType(VideoCapabilityMetadata(codecs = "dvhe.05.06")),
        )
        val chroma = CodecCapabilityPreflight.classify(
            metadata("HEVC Main 10 4:2:2").copy(chromaFormat = ChromaFormat.YUV_422),
            "video/hevc",
            listOf(decoder()),
        )
        assertEquals("UNSUPPORTED_CHROMA", chroma.reasonCode)
        assertEquals(CapabilityStatus.UNSUPPORTED, chroma.status)
    }

    @Test
    fun recognizesAvcHevcAndDolbyCodecSpellings() {
        for (codec in listOf("avc1.640028", "avc3.640028", "H.264")) {
            assertEquals("video/avc", CodecCapabilityPreflight.resolveMimeType(VideoCapabilityMetadata(codecs = codec)))
        }
        for (codec in listOf("hvc1.1.6.L120", "hev1.2.4.L153", "HEVC Main", "HEVC Main 10")) {
            assertEquals("video/hevc", CodecCapabilityPreflight.resolveMimeType(VideoCapabilityMetadata(codecs = codec)))
        }
    }
}
