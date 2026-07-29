package com.cameronamer.telegramdrive.nativeplayer

import androidx.media3.common.PlaybackException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import java.net.ConnectException
import java.net.SocketTimeoutException

class NativePlayerErrorMapperTest {
    @Test
    fun mapsAuthenticationRangeNotFoundAndServerStatuses() {
        assertEquals("authentication", NativePlayerErrorMapper.mapHttpStatus(401).category)
        assertEquals("authentication", NativePlayerErrorMapper.mapHttpStatus(403).category)
        assertEquals("server", NativePlayerErrorMapper.mapHttpStatus(404).category)
        assertEquals("server", NativePlayerErrorMapper.mapHttpStatus(416).category)
        assertEquals("server", NativePlayerErrorMapper.mapHttpStatus(503).category)
    }

    @Test
    fun mapsAudioOnlyCompatibilityFailureAsAudioCodec() {
        val tracks = TrackSupportFacts(
            hasVideoTracks = true,
            hasPlayableVideoTrack = true,
            hasAudioTracks = true,
            hasPlayableAudioTrack = false,
            primaryPlayback = PrimaryTrackPlayback.VIDEO_PLAYABLE_AUDIO_UNSUPPORTED,
        )
        val mapped = NativePlayerErrorMapper.map(
            PlaybackException("private details", null, PlaybackException.ERROR_CODE_DECODING_FORMAT_UNSUPPORTED),
            tracks,
        )
        assertEquals("audio-codec", mapped.category)
        assertFalse(mapped.message.contains("private details"))
    }

    @Test
    fun capabilityTurnsProbableProfileFailureIntoVideoCodecError() {
        val capability = VideoCapabilityResult.unknown("x", "x").copy(
            status = CapabilityStatus.UNSUPPORTED,
            codecFamily = VideoCodecFamily.HEVC,
            hevcProfile = HevcProfile.MAIN_10,
            reasonCode = "UNSUPPORTED_PROFILE",
        )
        val tracks = TrackSupportFacts(
            hasVideoTracks = true,
            hasPlayableVideoTrack = false,
            primaryPlayback = PrimaryTrackPlayback.NO_PLAYABLE_PRIMARY_TRACKS,
        )
        val mapped = NativePlayerErrorMapper.map(
            PlaybackException("private details", null, PlaybackException.ERROR_CODE_DECODER_INIT_FAILED),
            tracks,
            capability,
        )
        assertEquals("video-codec", mapped.category)
        assertEquals("UNSUPPORTED_VIDEO_PROFILE", mapped.code)
    }

    @Test
    fun decoderInitAndRuntimeRemainDistinct() {
        val init = NativePlayerErrorMapper.map(
            PlaybackException("x", null, PlaybackException.ERROR_CODE_DECODER_INIT_FAILED),
            TrackSupportFacts(),
        )
        val runtime = NativePlayerErrorMapper.map(
            PlaybackException("x", null, PlaybackException.ERROR_CODE_DECODING_FAILED),
            TrackSupportFacts(),
        )
        assertEquals("decoder-init", init.category)
        assertEquals("decoder-runtime", runtime.category)
    }

    @Test
    fun connectionRefusalAndTimeoutHaveSafeSpecificCodes() {
        val refused = NativePlayerErrorMapper.map(
            PlaybackException(
                "private loopback URL",
                ConnectException("Connection refused"),
                PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_FAILED,
            ),
            TrackSupportFacts(),
        )
        val timeout = NativePlayerErrorMapper.map(
            PlaybackException(
                "private loopback URL",
                SocketTimeoutException("timed out"),
                PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_TIMEOUT,
            ),
            TrackSupportFacts(),
        )
        assertEquals("CONNECTION_REFUSED", refused.code)
        assertEquals("CONNECT_TIMEOUT", timeout.code)
        assertFalse(refused.message.contains("URL"))
        assertFalse(timeout.message.contains("URL"))
    }
}
