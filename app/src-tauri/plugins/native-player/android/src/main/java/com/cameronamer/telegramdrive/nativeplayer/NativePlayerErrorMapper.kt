package com.cameronamer.telegramdrive.nativeplayer

import androidx.media3.common.PlaybackException
import androidx.media3.datasource.HttpDataSource
import java.io.InterruptedIOException
import java.net.ConnectException
import java.net.SocketTimeoutException

data class TrackSupportFacts(
    val hasVideoTrack: Boolean = false,
    val hasAudioTrack: Boolean = false,
    val unsupportedVideo: Boolean = false,
    val unsupportedAudio: Boolean = false,
)

object NativePlayerErrorMapper {
    fun map(error: PlaybackException, tracks: TrackSupportFacts): NativePlayerPublicError {
        findCause<HttpDataSource.InvalidResponseCodeException>(error)?.let {
            return mapHttpStatus(it.responseCode)
        }
        if (findCause<SocketTimeoutException>(error) != null) {
            return public("network", "TIMEOUT", "The local stream timed out. Try playback again.")
        }
        if (findCause<InterruptedIOException>(error) != null) {
            return public("network", "INTERRUPTED", "The media request was interrupted.")
        }
        if (findCause<ConnectException>(error) != null) {
            return public("server", "LOOPBACK_UNAVAILABLE", "The local streaming server is unavailable.")
        }

        return when (error.errorCode) {
            PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_FAILED,
            PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_TIMEOUT ->
                public("network", error.errorCodeName, "The local stream connection failed.")

            PlaybackException.ERROR_CODE_PARSING_CONTAINER_UNSUPPORTED ->
                public("container", error.errorCodeName, "This media container is not supported by Android.")

            PlaybackException.ERROR_CODE_PARSING_CONTAINER_MALFORMED ->
                public("container", error.errorCodeName, "The media container is malformed or corrupt.")

            PlaybackException.ERROR_CODE_DECODER_INIT_FAILED,
            PlaybackException.ERROR_CODE_DECODER_QUERY_FAILED ->
                public("decoder-init", error.errorCodeName, "Android could not initialize a hardware decoder for this media.")

            PlaybackException.ERROR_CODE_DECODING_FAILED ->
                public("decoder-runtime", error.errorCodeName, "The Android decoder failed while playing this media.")

            PlaybackException.ERROR_CODE_DECODING_FORMAT_UNSUPPORTED -> codecFailure(tracks, error.errorCodeName)
            else -> when {
                tracks.unsupportedAudio && !tracks.unsupportedVideo ->
                    public("audio-codec", error.errorCodeName, "The audio track is not supported on this device.")
                tracks.unsupportedVideo ->
                    public("video-codec", error.errorCodeName, "The video track is not supported on this device.")
                else -> public("unknown", error.errorCodeName, "Playback failed. The file may be unsupported or damaged.")
            }
        }
    }

    fun mapHttpStatus(status: Int): NativePlayerPublicError = when (status) {
        401, 403 -> public("authentication", "HTTP_$status", "The secure local stream session was rejected.")
        404 -> public("server", "HTTP_404", "The Telegram media message could not be found.")
        416 -> public("server", "HTTP_416", "The requested media byte range is no longer valid.")
        in 500..599 -> public("server", "HTTP_$status", "The local streaming server could not read this media.")
        else -> public("network", "HTTP_$status", "The local media request failed with HTTP $status.")
    }

    private fun codecFailure(tracks: TrackSupportFacts, code: String): NativePlayerPublicError =
        if (tracks.unsupportedAudio && !tracks.unsupportedVideo) {
            public("audio-codec", code, "The audio codec is not supported on this device.")
        } else {
            public("video-codec", code, "The video codec or profile is not supported on this device.")
        }

    private fun public(category: String, code: String, message: String) =
        NativePlayerPublicError(category, code, message)

    private inline fun <reified T : Throwable> findCause(error: Throwable): T? {
        var cause: Throwable? = error
        while (cause != null) {
            if (cause is T) return cause
            cause = cause.cause
        }
        return null
    }
}
