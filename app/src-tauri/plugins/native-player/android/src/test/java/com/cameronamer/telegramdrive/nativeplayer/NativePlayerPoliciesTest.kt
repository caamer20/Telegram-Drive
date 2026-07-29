package com.cameronamer.telegramdrive.nativeplayer

import androidx.media3.common.C
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NativePlayerPoliciesTest {
    private fun group(type: Int, supported: List<Boolean>, selected: List<Boolean> = supported.map { false }) =
        TrackGroupFacts(type, supported, selected)

    @Test
    fun supportedVideoIgnoresUnsupportedAlternativeVideo() {
        val facts = TrackSupportAnalyzer.analyze(listOf(
            group(C.TRACK_TYPE_VIDEO, listOf(true), listOf(true)),
            group(C.TRACK_TYPE_VIDEO, listOf(false)),
        ))
        assertTrue(facts.hasPlayableVideoTrack)
        assertTrue(facts.selectedVideoTrackSupported)
        assertTrue(facts.hasUnsupportedAlternativeVideoTrack)
        assertEquals(PrimaryTrackPlayback.VIDEO_ONLY_PLAYABLE, facts.primaryPlayback)
    }

    @Test
    fun distinguishesSupportedVideoUnsupportedAudioAndReverse() {
        val videoOnly = TrackSupportAnalyzer.analyze(listOf(
            group(C.TRACK_TYPE_VIDEO, listOf(true), listOf(true)),
            group(C.TRACK_TYPE_AUDIO, listOf(false), listOf(true)),
        ))
        assertEquals(PrimaryTrackPlayback.VIDEO_PLAYABLE_AUDIO_UNSUPPORTED, videoOnly.primaryPlayback)
        val audioOnly = TrackSupportAnalyzer.analyze(listOf(
            group(C.TRACK_TYPE_VIDEO, listOf(false), listOf(true)),
            group(C.TRACK_TYPE_AUDIO, listOf(true), listOf(true)),
        ))
        assertEquals(PrimaryTrackPlayback.AUDIO_PLAYABLE_VIDEO_UNSUPPORTED, audioOnly.primaryPlayback)
    }

    @Test
    fun multipleAudioTracksArePlayableWhenAnyOneIsSupported() {
        val facts = TrackSupportAnalyzer.analyze(listOf(
            group(C.TRACK_TYPE_AUDIO, listOf(false, true), listOf(false, true)),
        ))
        assertTrue(facts.hasPlayableAudioTrack)
        assertTrue(facts.selectedAudioTrackSupported)
        assertTrue(facts.hasUnsupportedAlternativeAudioTrack)
        assertEquals(PrimaryTrackPlayback.AUDIO_ONLY_PLAYABLE, facts.primaryPlayback)
    }

    @Test
    fun identifiesNoPlayablePrimaryTracks() {
        val facts = TrackSupportAnalyzer.analyze(listOf(
            group(C.TRACK_TYPE_VIDEO, listOf(false)),
            group(C.TRACK_TYPE_AUDIO, listOf(false)),
        ))
        assertEquals(PrimaryTrackPlayback.NO_PLAYABLE_PRIMARY_TRACKS, facts.primaryPlayback)
    }

    @Test
    fun retryEligibilityIsBoundedAndPermanentErrorsStayClosed() {
        assertTrue(NativePlaybackErrorPolicy.isRetryEligible(
            NativePlayerPublicError("network", "READ_TIMEOUT", "safe"), 0,
        ))
        assertTrue(NativePlaybackErrorPolicy.isRetryEligible(
            NativePlayerPublicError("server", "HTTP_503", "safe"), 1,
        ))
        assertTrue(NativePlaybackErrorPolicy.isRetryEligible(
            NativePlayerPublicError("decoder-init", "DECODER_INIT_FAILED", "safe"), 0,
        ))
        assertFalse(NativePlaybackErrorPolicy.isRetryEligible(
            NativePlayerPublicError("authentication", "HTTP_401", "safe"), 0,
        ))
        assertFalse(NativePlaybackErrorPolicy.isRetryEligible(
            NativePlayerPublicError("container", "CORRUPT_MEDIA", "safe"), 0,
        ))
        assertFalse(NativePlaybackErrorPolicy.isRetryEligible(
            NativePlayerPublicError("network", "READ_TIMEOUT", "safe"),
            NativePlaybackErrorPolicy.MAX_MANUAL_RETRIES,
        ))
    }

    @Test
    fun pipRequiresPlayingVideoAndClampsExtremeRatios() {
        assertTrue(PictureInPicturePolicy.isEligible(true, true, false))
        assertFalse(PictureInPicturePolicy.isEligible(false, true, false))
        assertFalse(PictureInPicturePolicy.isEligible(true, false, false))
        assertFalse(PictureInPicturePolicy.isEligible(true, true, true))
        assertEquals(SafeAspectRatio(16, 9), PictureInPicturePolicy.sanitizeAspectRatio(1920, 1080))
        assertEquals(SafeAspectRatio(239, 100), PictureInPicturePolicy.sanitizeAspectRatio(4000, 1000))
        assertEquals(SafeAspectRatio(100, 239), PictureInPicturePolicy.sanitizeAspectRatio(1000, 4000))
        assertNull(PictureInPicturePolicy.sanitizeAspectRatio(0, 1080))
    }

    @Test
    fun eventsOnlyEmitMeaningfulTransitionsAndNeverPositionTicks() {
        assertTrue(NativeEventTransitionPolicy.shouldEmit("idle", false, "buffering", false))
        assertTrue(NativeEventTransitionPolicy.shouldEmit("ready", false, "playing", true))
        assertFalse(NativeEventTransitionPolicy.shouldEmit("playing", true, "playing", true))
        assertFalse(NativeEventTransitionPolicy.shouldEmit("idle", false, "idle", false))
        assertFalse(NativeEventTransitionPolicy.shouldEmit("playing", true, "idle", false))
    }

    @Test
    fun mimeHintsStayAdvisoryAndGenericBinaryIsNotForced() {
        assertEquals("video/mp4", NativePlayerMimePolicy.sanitizeHint("video/mp4; charset=binary"))
        assertEquals("video/x-matroska", NativePlayerMimePolicy.sanitizeHint("video/x-matroska"))
        assertEquals("video/webm", NativePlayerMimePolicy.sanitizeHint("video/webm"))
        assertNull(NativePlayerMimePolicy.sanitizeHint("application/octet-stream"))
        assertEquals("application/vnd.apple.mpegurl", NativePlayerMimePolicy.sanitizeHint("application/vnd.apple.mpegurl"))
        assertEquals("application/x-mpegURL", NativePlayerMimePolicy.sanitizeHint("application/x-mpegURL"))
        assertFalse(NativePlayerMimePolicy.shouldApplyToMediaItem("video/mp4"))
        assertTrue(NativePlayerMimePolicy.shouldApplyToMediaItem("application/x-mpegURL"))
    }

    @Test
    fun restoreValidationExpiresAndRejectsMalformedIdentity() {
        val restore = PendingNativePlayerRestore(null, 7, "Movie", "movie.mkv", "video/mp4", 10, true)
        val created = 1_000L
        assertTrue(PendingNativePlayerRestoreStore.isFreshAndValid(restore, created, created + 1))
        assertFalse(PendingNativePlayerRestoreStore.isFreshAndValid(
            restore, created, created + PendingNativePlayerRestoreStore.RESTORE_TTL_MS + 1,
        ))
        assertFalse(PendingNativePlayerRestoreStore.isFreshAndValid(
            restore.copy(messageId = 0), created, created + 1,
        ))
    }
}
