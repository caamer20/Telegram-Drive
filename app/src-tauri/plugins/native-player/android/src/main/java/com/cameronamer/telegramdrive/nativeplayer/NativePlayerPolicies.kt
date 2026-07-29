package com.cameronamer.telegramdrive.nativeplayer

import androidx.media3.common.C

data class TrackGroupFacts(
    val type: Int,
    val supported: List<Boolean>,
    val selected: List<Boolean>,
)

enum class PrimaryTrackPlayback {
    AUDIO_VIDEO_PLAYABLE,
    VIDEO_ONLY_PLAYABLE,
    AUDIO_ONLY_PLAYABLE,
    VIDEO_PLAYABLE_AUDIO_UNSUPPORTED,
    AUDIO_PLAYABLE_VIDEO_UNSUPPORTED,
    NO_PLAYABLE_PRIMARY_TRACKS,
    UNKNOWN,
}

data class TrackSupportFacts(
    val hasVideoTracks: Boolean = false,
    val hasPlayableVideoTrack: Boolean = false,
    val hasSelectedVideoTrack: Boolean = false,
    val selectedVideoTrackSupported: Boolean = false,
    val hasUnsupportedAlternativeVideoTrack: Boolean = false,
    val hasAudioTracks: Boolean = false,
    val hasPlayableAudioTrack: Boolean = false,
    val hasSelectedAudioTrack: Boolean = false,
    val selectedAudioTrackSupported: Boolean = false,
    val hasUnsupportedAlternativeAudioTrack: Boolean = false,
    val primaryPlayback: PrimaryTrackPlayback = PrimaryTrackPlayback.UNKNOWN,
)

object TrackSupportAnalyzer {
    fun analyze(groups: List<TrackGroupFacts>): TrackSupportFacts {
        val video = analyzeType(groups.filter { it.type == C.TRACK_TYPE_VIDEO })
        val audio = analyzeType(groups.filter { it.type == C.TRACK_TYPE_AUDIO })
        val primary = when {
            video.playable && audio.playable -> PrimaryTrackPlayback.AUDIO_VIDEO_PLAYABLE
            video.playable && !audio.present -> PrimaryTrackPlayback.VIDEO_ONLY_PLAYABLE
            audio.playable && !video.present -> PrimaryTrackPlayback.AUDIO_ONLY_PLAYABLE
            video.playable && audio.present && !audio.playable -> PrimaryTrackPlayback.VIDEO_PLAYABLE_AUDIO_UNSUPPORTED
            audio.playable && video.present && !video.playable -> PrimaryTrackPlayback.AUDIO_PLAYABLE_VIDEO_UNSUPPORTED
            video.present || audio.present -> PrimaryTrackPlayback.NO_PLAYABLE_PRIMARY_TRACKS
            else -> PrimaryTrackPlayback.UNKNOWN
        }
        return TrackSupportFacts(
            hasVideoTracks = video.present,
            hasPlayableVideoTrack = video.playable,
            hasSelectedVideoTrack = video.selected,
            selectedVideoTrackSupported = video.selectedSupported,
            hasUnsupportedAlternativeVideoTrack = video.unsupportedAlternative,
            hasAudioTracks = audio.present,
            hasPlayableAudioTrack = audio.playable,
            hasSelectedAudioTrack = audio.selected,
            selectedAudioTrackSupported = audio.selectedSupported,
            hasUnsupportedAlternativeAudioTrack = audio.unsupportedAlternative,
            primaryPlayback = primary,
        )
    }

    private fun analyzeType(groups: List<TrackGroupFacts>): TypeFacts {
        val tracks = groups.flatMap { group ->
            val size = minOf(group.supported.size, group.selected.size)
            (0 until size).map { index -> group.supported[index] to group.selected[index] }
        }
        val selected = tracks.any { it.second }
        return TypeFacts(
            present = tracks.isNotEmpty(),
            playable = tracks.any { it.first },
            selected = selected,
            selectedSupported = tracks.any { it.first && it.second },
            unsupportedAlternative = tracks.any { !it.first && !it.second },
        )
    }

    private data class TypeFacts(
        val present: Boolean,
        val playable: Boolean,
        val selected: Boolean,
        val selectedSupported: Boolean,
        val unsupportedAlternative: Boolean,
    )
}

data class NativeErrorOverlayState(
    val title: String,
    val message: String,
    val canRetry: Boolean,
)

object NativePlaybackErrorPolicy {
    const val MAX_MANUAL_RETRIES = 2

    fun overlay(error: NativePlayerPublicError, retryCount: Int): NativeErrorOverlayState {
        val title = when (error.category) {
            "network" -> "Playback interrupted"
            "authentication" -> "Playback session expired"
            "server" -> "Local stream unavailable"
            "container" -> "Media cannot be opened"
            "video-codec" -> "Video format not supported"
            "audio-codec" -> "Audio format not supported"
            "decoder-init" -> "Decoder could not start"
            "decoder-runtime" -> "Decoder stopped"
            else -> "Playback failed"
        }
        return NativeErrorOverlayState(title, error.message, isRetryEligible(error, retryCount))
    }

    fun isRetryEligible(error: NativePlayerPublicError, retryCount: Int): Boolean {
        if (retryCount >= MAX_MANUAL_RETRIES) return false
        return when (error.category) {
            "network" -> true
            "decoder-init" -> true
            "server" -> error.code == "LOOPBACK_UNAVAILABLE" ||
                error.code == "CONNECTION_REFUSED" ||
                error.code == "HTTP_500" || error.code == "HTTP_502" ||
                error.code == "HTTP_503" || error.code == "HTTP_504"
            else -> false
        }
    }
}

object NativeEventTransitionPolicy {
    private val publicStates = setOf("buffering", "ready", "playing", "paused", "ended", "error")

    fun shouldEmit(
        previousState: String,
        previousIsPlaying: Boolean,
        nextState: String,
        nextIsPlaying: Boolean,
    ): Boolean = nextState in publicStates &&
        (nextState != previousState || nextIsPlaying != previousIsPlaying)
}

data class SafeAspectRatio(val numerator: Int, val denominator: Int)

object PictureInPicturePolicy {
    private const val MAX_RATIO = 2.39
    private const val MIN_RATIO = 1.0 / MAX_RATIO

    fun isEligible(hasPlayableVideo: Boolean, isPlaying: Boolean, hasError: Boolean): Boolean =
        hasPlayableVideo && isPlaying && !hasError

    fun sanitizeAspectRatio(width: Int, height: Int): SafeAspectRatio? {
        if (width <= 0 || height <= 0) return null
        val ratio = width.toDouble() / height.toDouble()
        if (!ratio.isFinite()) return null
        if (ratio > MAX_RATIO) return SafeAspectRatio(239, 100)
        if (ratio < MIN_RATIO) return SafeAspectRatio(100, 239)
        val gcd = gcd(width, height)
        return SafeAspectRatio(width / gcd, height / gcd)
    }

    private tailrec fun gcd(left: Int, right: Int): Int =
        if (right == 0) left.coerceAtLeast(1) else gcd(right, left % right)
}

object NativePlayerMimePolicy {
    fun sanitizeHint(value: String?): String? {
        val mime = value?.substringBefore(';')?.trim()?.lowercase()?.takeIf(String::isNotEmpty)
            ?: return null
        if (mime == "application/octet-stream") return null
        return when (mime) {
            "video/mp4", "video/x-matroska", "video/webm",
            "audio/mp4", "audio/mpeg", "audio/webm" -> mime
            "application/vnd.apple.mpegurl" -> "application/vnd.apple.mpegurl"
            "application/x-mpegurl" -> "application/x-mpegURL"
            else -> null
        }
    }

    fun shouldApplyToMediaItem(mime: String?): Boolean = mime?.lowercase() in setOf(
        "application/vnd.apple.mpegurl",
        "application/x-mpegurl",
    )
}
