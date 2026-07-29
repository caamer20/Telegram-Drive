package com.cameronamer.telegramdrive.nativeplayer

import androidx.media3.common.C
import androidx.media3.common.MimeTypes
import androidx.media3.common.ParserException
import androidx.media3.common.PlaybackException
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.ExoPlaybackException
import androidx.media3.exoplayer.mediacodec.MediaCodecDecoderException
import androidx.media3.exoplayer.mediacodec.MediaCodecRenderer
import androidx.media3.exoplayer.source.UnrecognizedInputFormatException
import androidx.media3.exoplayer.video.MediaCodecVideoDecoderException
import java.io.InterruptedIOException
import java.net.ConnectException
import java.net.SocketTimeoutException

@androidx.annotation.OptIn(UnstableApi::class)
object NativePlayerErrorMapper {
    fun map(
        error: PlaybackException,
        tracks: TrackSupportFacts,
        capability: VideoCapabilityResult? = null,
    ): NativePlayerPublicError {
        findCause<HttpDataSource.InvalidResponseCodeException>(error)?.let {
            return mapHttpStatus(it.responseCode)
        }
        findCause<ConnectException>(error)?.let {
            return public(
                "server",
                if (it.message?.contains("refused", true) == true) "CONNECTION_REFUSED" else "LOOPBACK_UNAVAILABLE",
                "The private local streaming server is unavailable.",
            )
        }
        findCause<SocketTimeoutException>(error)?.let {
            val connectTimeout = error.errorCode == PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_TIMEOUT
            return public(
                "network",
                if (connectTimeout) "CONNECT_TIMEOUT" else "READ_TIMEOUT",
                if (connectTimeout) "The local stream connection timed out." else "The local stream stopped responding.",
            )
        }
        if (findCause<InterruptedIOException>(error) != null) {
            return public("network", "REQUEST_INTERRUPTED", "The media request was interrupted.")
        }
        if (findCause<UnrecognizedInputFormatException>(error) != null) {
            return public("container", "EXTRACTOR_UNSUPPORTED", "Android could not recognize this media container.")
        }
        findCause<ParserException>(error)?.let { parser ->
            return if (parser.contentIsMalformed) {
                public("container", "CORRUPT_MEDIA", "The media container is malformed or corrupt.")
            } else {
                public("container", "EXTRACTOR_UNSUPPORTED", "This media container uses unsupported features.")
            }
        }

        val exo = error as? ExoPlaybackException
        val rendererType = exo?.rendererFormat?.sampleMimeType?.let(MimeTypes::getTrackType)
            ?: C.TRACK_TYPE_UNKNOWN
        val decoderInit = findCause<MediaCodecRenderer.DecoderInitializationException>(error)
        if (decoderInit != null || error.errorCode in setOf(
                PlaybackException.ERROR_CODE_DECODER_INIT_FAILED,
                PlaybackException.ERROR_CODE_DECODER_QUERY_FAILED,
            )
        ) {
            if (rendererType == C.TRACK_TYPE_VIDEO ||
                (rendererType == C.TRACK_TYPE_UNKNOWN && tracks.hasVideoTracks && !tracks.hasPlayableVideoTrack)
            ) {
                capabilityFailure(capability)?.let { return it }
            }
            return public(
                "decoder-init",
                if (rendererType == C.TRACK_TYPE_AUDIO) "AUDIO_DECODER_INIT_FAILED" else "DECODER_INIT_FAILED",
                "Android could not initialize a decoder for the selected media track.",
            )
        }

        if (findCause<MediaCodecVideoDecoderException>(error) != null) {
            return public("decoder-runtime", "VIDEO_DECODER_RUNTIME_FAILED", "The video decoder stopped while playing this media.")
        }
        if (findCause<MediaCodecDecoderException>(error) != null ||
            error.errorCode == PlaybackException.ERROR_CODE_DECODING_FAILED
        ) {
            return public(
                "decoder-runtime",
                if (rendererType == C.TRACK_TYPE_AUDIO) "AUDIO_DECODER_RUNTIME_FAILED" else "DECODER_RUNTIME_FAILED",
                "The Android decoder stopped while playing this media.",
            )
        }

        return when (error.errorCode) {
            PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_TIMEOUT ->
                public("network", "CONNECT_TIMEOUT", "The local stream connection timed out.")
            PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_FAILED ->
                public("network", "NETWORK_CONNECTION_FAILED", "The local stream connection failed.")
            PlaybackException.ERROR_CODE_IO_READ_POSITION_OUT_OF_RANGE ->
                public("server", "HTTP_416", "The requested media byte range is no longer valid.")
            PlaybackException.ERROR_CODE_PARSING_CONTAINER_UNSUPPORTED ->
                public("container", "EXTRACTOR_UNSUPPORTED", "This media container is not supported by Android.")
            PlaybackException.ERROR_CODE_PARSING_CONTAINER_MALFORMED ->
                public("container", "CORRUPT_MEDIA", "The media container is malformed or corrupt.")
            PlaybackException.ERROR_CODE_DECODING_FORMAT_UNSUPPORTED ->
                formatUnsupported(rendererType, tracks, capability)
            else -> trackAwareFallback(error.errorCodeName, tracks, capability)
        }
    }

    fun mapHttpStatus(status: Int): NativePlayerPublicError = when (status) {
        401 -> public("authentication", "HTTP_401", "The private playback session is no longer authorized.")
        403 -> public("authentication", "HTTP_403", "The private playback session was rejected.")
        404 -> public("server", "HTTP_404", "The Telegram media message could not be found.")
        416 -> public("server", "HTTP_416", "The requested media byte range is no longer valid.")
        in 500..599 -> public("server", "HTTP_$status", "The local streaming server could not read this media.")
        else -> public("network", "HTTP_$status", "The local media request failed with HTTP $status.")
    }

    fun noPlayableTracks(tracks: TrackSupportFacts, capability: VideoCapabilityResult?): NativePlayerPublicError =
        trackAwareFallback("NO_PLAYABLE_PRIMARY_TRACKS", tracks, capability)

    private fun formatUnsupported(
        rendererType: Int,
        tracks: TrackSupportFacts,
        capability: VideoCapabilityResult?,
    ): NativePlayerPublicError = when (rendererType) {
        C.TRACK_TYPE_AUDIO -> public("audio-codec", "UNSUPPORTED_AUDIO_MIME", "The selected audio codec is not supported on this device.")
        C.TRACK_TYPE_VIDEO -> capabilityFailure(capability)
            ?: public("video-codec", "UNSUPPORTED_VIDEO_MIME", "The selected video codec is not supported on this device.")
        else -> trackAwareFallback("UNSUPPORTED_FORMAT", tracks, capability)
    }

    private fun trackAwareFallback(
        code: String,
        tracks: TrackSupportFacts,
        capability: VideoCapabilityResult?,
    ): NativePlayerPublicError {
        if (tracks.hasSelectedAudioTrack && !tracks.selectedAudioTrackSupported) {
            return public("audio-codec", "UNSUPPORTED_AUDIO_CODEC", "The selected audio track is not supported on this device.")
        }
        if (tracks.hasSelectedVideoTrack && !tracks.selectedVideoTrackSupported) {
            return capabilityFailure(capability)
                ?: public("video-codec", "UNSUPPORTED_VIDEO_CODEC", "The selected video track is not supported on this device.")
        }
        return when (tracks.primaryPlayback) {
            PrimaryTrackPlayback.VIDEO_PLAYABLE_AUDIO_UNSUPPORTED ->
                public("audio-codec", "UNSUPPORTED_AUDIO_CODEC", "No playable audio track is available on this device.")
            PrimaryTrackPlayback.AUDIO_PLAYABLE_VIDEO_UNSUPPORTED -> capabilityFailure(capability)
                ?: public("video-codec", "UNSUPPORTED_VIDEO_CODEC", "No playable video track is available on this device.")
            PrimaryTrackPlayback.NO_PLAYABLE_PRIMARY_TRACKS -> when {
                tracks.hasVideoTracks -> capabilityFailure(capability)
                    ?: public("video-codec", "NO_PLAYABLE_VIDEO", "No playable video track is available on this device.")
                tracks.hasAudioTracks -> public("audio-codec", "NO_PLAYABLE_AUDIO", "No playable audio track is available on this device.")
                else -> public("container", "NO_PRIMARY_TRACKS", "The media contains no playable audio or video tracks.")
            }
            else -> public("unknown", code, "Playback failed. The media may be unsupported or damaged.")
        }
    }

    private fun capabilityFailure(capability: VideoCapabilityResult?): NativePlayerPublicError? {
        if (capability?.status != CapabilityStatus.UNSUPPORTED) return null
        return when (capability.reasonCode) {
            "UNSUPPORTED_HEVC_PROFILE", "UNSUPPORTED_PROFILE" -> public(
                "video-codec",
                "UNSUPPORTED_VIDEO_PROFILE",
                "This device does not report support for the extracted video profile.",
            )
            "UNSUPPORTED_LEVEL" -> public(
                "video-codec",
                "UNSUPPORTED_VIDEO_LEVEL",
                "This device does not report support for the extracted video level.",
            )
            "UNSUPPORTED_RESOLUTION" -> public(
                "video-codec",
                "UNSUPPORTED_VIDEO_RESOLUTION",
                "The extracted video resolution exceeds this device's reported decoder limits.",
            )
            "UNSUPPORTED_FRAME_RATE" -> public(
                "video-codec",
                "UNSUPPORTED_VIDEO_RATE",
                "The extracted resolution and frame rate exceed this device's reported decoder limits.",
            )
            "UNSUPPORTED_BITRATE" -> public(
                "video-codec",
                "UNSUPPORTED_VIDEO_BITRATE",
                "The extracted bitrate exceeds this device's reported decoder limits.",
            )
            "UNSUPPORTED_CHROMA" -> public(
                "video-codec",
                "UNSUPPORTED_VIDEO_CHROMA",
                "The extracted chroma format is not supported for Android playback.",
            )
            "NO_DECODER" -> public(
                "video-codec",
                "UNSUPPORTED_VIDEO_MIME",
                "No Android decoder reports support for the extracted video codec.",
            )
            else -> null
        }
    }

    private fun public(category: String, code: String, message: String) =
        NativePlayerPublicError(category, code, message)

    private inline fun <reified T : Throwable> findCause(error: Throwable): T? {
        var cause: Throwable? = error
        val seen = mutableSetOf<Throwable>()
        while (cause != null && seen.add(cause)) {
            if (cause is T) return cause
            cause = cause.cause
        }
        return null
    }
}
