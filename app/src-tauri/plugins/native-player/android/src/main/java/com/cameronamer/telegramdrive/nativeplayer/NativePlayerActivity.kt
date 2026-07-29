package com.cameronamer.telegramdrive.nativeplayer

import android.app.Activity
import android.app.PictureInPictureParams
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Rational
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.activity.OnBackPressedCallback
import androidx.annotation.RequiresApi
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.Tracks
import androidx.media3.common.VideoSize
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.exoplayer.trackselection.DefaultTrackSelector
import androidx.media3.session.MediaSession
import androidx.media3.ui.PlayerView
import java.util.concurrent.atomic.AtomicBoolean

@androidx.annotation.OptIn(markerClass = [UnstableApi::class])
class NativePlayerActivity : AppCompatActivity(), Player.Listener {
    private var sessionId: String? = null
    private var session: NativePlayerSession? = null
    private var playerView: PlayerView? = null
    private var player: ExoPlayer? = null
    private var mediaSession: MediaSession? = null
    private var trackSelector: DefaultTrackSelector? = null
    private var hasPlayableVideo = false
    private var trackFacts = TrackSupportFacts()
    private var completed = false
    private var exitReason = "back"
    private var playbackError: NativePlayerPublicError? = null
    private var resumeOnForeground = false
    private var restoredPositionMs = 0L
    private var restoredPlayWhenReady: Boolean? = null
    private val finishing = AtomicBoolean(false)
    private val mainHandler = Handler(Looper.getMainLooper())
    @Volatile private var latestSnapshot = NativePlaybackSnapshot()

    private val snapshotTicker = object : Runnable {
        override fun run() {
            updateSnapshot(emitEvent = false)
            if (!isFinishing && !isDestroyed) mainHandler.postDelayed(this, 1_000)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        sessionId = savedInstanceState?.getString(STATE_SESSION_ID)
            ?: intent.getStringExtra(EXTRA_SESSION_ID)
        session = NativePlayerSessionStore.get(sessionId)
        if (session == null) {
            PendingNativePlayerRestoreStore.save(
                this,
                intent,
                savedInstanceState?.getLong(STATE_POSITION_MS) ?: 0,
                savedInstanceState?.getBoolean(STATE_PLAY_WHEN_READY)
                    ?: intent.getBooleanExtra(EXTRA_AUTOPLAY, true),
            )
            finishWithResult(
                NativePlayerPublicError(
                    "server",
                    "SESSION_LOST",
                    "The private playback session was lost. Return to Telegram Drive and try again.",
                ),
            )
            return
        }

        restoredPositionMs = savedInstanceState?.getLong(STATE_POSITION_MS)
            ?: session!!.args.startPositionMs
        restoredPlayWhenReady = if (savedInstanceState?.containsKey(STATE_PLAY_WHEN_READY) == true) {
            savedInstanceState.getBoolean(STATE_PLAY_WHEN_READY)
        } else null

        configureFullscreen()
        playerView = PlayerView(this).apply {
            useController = true
            controllerAutoShow = true
            controllerHideOnTouch = true
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
        }
        setContentView(FrameLayout(this).apply {
            setBackgroundColor(android.graphics.Color.BLACK)
            addView(playerView)
        })

        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() = finishWithResult()
        })

        NativePlayerActivityRegistry.register(this)
        initializePlayer()
        mainHandler.post(snapshotTicker)
    }

    private fun configureFullscreen() {
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, window.decorView).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
    }

    private fun initializePlayer() {
        val args = session?.args ?: return
        // Advisory only. Playback is always attempted even when device-reported
        // limits look insufficient because MediaCodec reports can be incomplete.
        CodecCapabilityPreflight.inspect(
            VideoCapabilityMetadata(
                args.codec,
                args.width,
                args.height,
                args.frameRate,
                args.bitrate,
                args.bitDepth,
                args.hdr,
            ),
        )

        val httpFactory = DefaultHttpDataSource.Factory()
            .setConnectTimeoutMs(15_000)
            .setReadTimeoutMs(30_000)
            .setAllowCrossProtocolRedirects(false)
            .setDefaultRequestProperties(mapOf("Authorization" to "Bearer ${args.authorizationToken}"))
        val renderersFactory = DefaultRenderersFactory(this).setEnableDecoderFallback(true)
        trackSelector = DefaultTrackSelector(this)
        val exoPlayer = ExoPlayer.Builder(this, renderersFactory)
            .setTrackSelector(trackSelector!!)
            .setMediaSourceFactory(DefaultMediaSourceFactory(this).setDataSourceFactory(httpFactory))
            .build()
        player = exoPlayer
        playerView?.player = exoPlayer
        exoPlayer.addListener(this)
        exoPlayer.setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(C.USAGE_MEDIA)
                .setContentType(C.AUDIO_CONTENT_TYPE_MOVIE)
                .build(),
            true,
        )
        val itemBuilder = MediaItem.Builder().setUri(args.streamUrl).setMediaId(cacheKey(args))
        sanitizeMimeHint(args.mimeType)?.let(itemBuilder::setMimeType)
        exoPlayer.setMediaItem(itemBuilder.build(), restoredPositionMs.coerceAtLeast(0))
        exoPlayer.playWhenReady = restoredPlayWhenReady ?: args.autoplay
        mediaSession = MediaSession.Builder(this, exoPlayer).build()
        exoPlayer.prepare()
        updateSnapshot(emitEvent = true)
    }

    private fun cacheKey(args: OpenNativePlayerArgs): String =
        "telegram:${args.folderId ?: "home"}:${args.messageId}"

    private fun sanitizeMimeHint(value: String?): String? {
        val mime = value?.substringBefore(';')?.trim()?.lowercase() ?: return null
        return mime.takeIf {
            (it.startsWith("video/") || it.startsWith("audio/") || it == "application/vnd.apple.mpegurl") &&
                !it.contains("://")
        }
    }

    override fun onPlaybackStateChanged(playbackState: Int) {
        if (playbackState == Player.STATE_ENDED) {
            completed = true
            exitReason = "ended"
        }
        updateSnapshot(emitEvent = true)
    }

    override fun onIsPlayingChanged(isPlaying: Boolean) {
        updateSnapshot(emitEvent = true)
    }

    override fun onTracksChanged(tracks: Tracks) {
        var hasVideo = false
        var hasAudio = false
        var unsupportedVideo = false
        var unsupportedAudio = false
        tracks.groups.forEach { group ->
            val supported = (0 until group.length).any(group::isTrackSupported)
            when (group.type) {
                C.TRACK_TYPE_VIDEO -> {
                    hasVideo = true
                    if (!supported) unsupportedVideo = true else hasPlayableVideo = true
                }
                C.TRACK_TYPE_AUDIO -> {
                    hasAudio = true
                    if (!supported) unsupportedAudio = true
                }
            }
        }
        trackFacts = TrackSupportFacts(hasVideo, hasAudio, unsupportedVideo, unsupportedAudio)
    }

    override fun onVideoSizeChanged(videoSize: VideoSize) {
        if (videoSize.width > 0 && videoSize.height > 0) updatePictureInPicture(videoSize.width, videoSize.height)
    }

    override fun onPlayerError(error: PlaybackException) {
        playbackError = NativePlayerErrorMapper.map(error, trackFacts)
        exitReason = "error"
        updateSnapshot(emitEvent = true)
    }

    override fun onStart() {
        super.onStart()
        if (resumeOnForeground && !isInPictureInPictureMode) {
            player?.play()
            resumeOnForeground = false
        }
    }

    override fun onStop() {
        if (!isInPictureInPictureMode) {
            resumeOnForeground = player?.isPlaying == true
            player?.pause()
        }
        super.onStop()
    }

    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O &&
            playbackError == null && hasPlayableVideo && player?.isPlaying == true
        ) {
            try {
                enterPictureInPictureMode(currentPictureInPictureParams())
            } catch (_: IllegalArgumentException) {
                // Device-specific PiP ratio/feature rejection is non-fatal.
            }
        }
    }

    override fun onPictureInPictureModeChanged(inPictureInPictureMode: Boolean, newConfig: Configuration) {
        super.onPictureInPictureModeChanged(inPictureInPictureMode, newConfig)
        playerView?.useController = !inPictureInPictureMode
        if (!inPictureInPictureMode) playerView?.showController()
    }

    private fun updatePictureInPicture(width: Int, height: Int) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        try {
            setPictureInPictureParams(
                PictureInPictureParams.Builder().setAspectRatio(Rational(width, height)).build(),
            )
        } catch (_: IllegalArgumentException) {
            // Invalid extreme aspect ratios are ignored.
        }
    }

    @RequiresApi(Build.VERSION_CODES.O)
    private fun currentPictureInPictureParams(): PictureInPictureParams {
        val size = player?.videoSize
        return if (size != null && size.width > 0 && size.height > 0) {
            PictureInPictureParams.Builder().setAspectRatio(Rational(size.width, size.height)).build()
        } else {
            PictureInPictureParams.Builder().build()
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString(STATE_SESSION_ID, sessionId)
        outState.putLong(STATE_POSITION_MS, safePosition())
        outState.putBoolean(STATE_PLAY_WHEN_READY, player?.playWhenReady ?: false)
        super.onSaveInstanceState(outState)
    }

    fun finishFromExternal() {
        exitReason = "external"
        finishWithResult()
    }

    fun playbackSnapshot(): NativePlaybackSnapshot = latestSnapshot

    private fun updateSnapshot(emitEvent: Boolean) {
        val exoPlayer = player
        val state = when {
            playbackError != null -> "error"
            exoPlayer == null -> "idle"
            exoPlayer.playbackState == Player.STATE_BUFFERING -> "buffering"
            exoPlayer.playbackState == Player.STATE_READY -> "ready"
            exoPlayer.playbackState == Player.STATE_ENDED -> "ended"
            else -> "idle"
        }
        val snapshot = NativePlaybackSnapshot(
            state,
            exoPlayer?.isPlaying == true,
            safePosition(),
            safeDuration(),
        )
        val transitioned = state != latestSnapshot.state || snapshot.isPlaying != latestSnapshot.isPlaying
        latestSnapshot = snapshot
        if (emitEvent && transitioned) session?.stateListener?.invoke(snapshot)
    }

    private fun safePosition(): Long = player?.currentPosition?.takeIf { it != C.TIME_UNSET }?.coerceAtLeast(0) ?: 0
    private fun safeDuration(): Long = player?.duration?.takeIf { it != C.TIME_UNSET }?.coerceAtLeast(0) ?: 0

    private fun finishWithResult(error: NativePlayerPublicError? = playbackError) {
        if (!finishing.compareAndSet(false, true)) return
        val result = NativePlayerResultData(
            positionMs = safePosition(),
            durationMs = safeDuration(),
            completed = completed,
            exitReason = if (error != null) "error" else exitReason,
            error = error,
        )
        setResult(Activity.RESULT_OK, NativePlayerResultCodec.toIntent(result))
        finish()
    }

    override fun onDestroy() {
        mainHandler.removeCallbacksAndMessages(null)
        NativePlayerActivityRegistry.clear(this)
        playerView?.player = null
        playerView = null
        mediaSession?.release()
        mediaSession = null
        player?.removeListener(this)
        player?.release()
        player = null
        trackSelector = null
        session = null
        super.onDestroy()
    }

    companion object {
        const val EXTRA_SESSION_ID = "com.cameronamer.telegramdrive.nativeplayer.SESSION_ID"
        const val EXTRA_FOLDER_ID = "com.cameronamer.telegramdrive.nativeplayer.FOLDER_ID"
        const val EXTRA_MESSAGE_ID = "com.cameronamer.telegramdrive.nativeplayer.MESSAGE_ID"
        const val EXTRA_TITLE = "com.cameronamer.telegramdrive.nativeplayer.TITLE"
        const val EXTRA_FILE_NAME = "com.cameronamer.telegramdrive.nativeplayer.FILE_NAME"
        const val EXTRA_MIME_TYPE = "com.cameronamer.telegramdrive.nativeplayer.MIME_TYPE"
        const val EXTRA_AUTOPLAY = "com.cameronamer.telegramdrive.nativeplayer.AUTOPLAY"
        private const val STATE_SESSION_ID = "nativePlayer.sessionId"
        private const val STATE_POSITION_MS = "nativePlayer.positionMs"
        private const val STATE_PLAY_WHEN_READY = "nativePlayer.playWhenReady"
    }
}
