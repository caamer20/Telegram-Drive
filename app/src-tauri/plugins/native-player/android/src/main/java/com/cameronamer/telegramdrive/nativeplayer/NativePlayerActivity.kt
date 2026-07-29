package com.cameronamer.telegramdrive.nativeplayer

import android.app.Activity
import android.app.PictureInPictureParams
import android.content.res.Configuration
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.util.Rational
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
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
    private var rootView: FrameLayout? = null
    private var playerView: PlayerView? = null
    private var errorOverlay: View? = null
    private var player: ExoPlayer? = null
    private var mediaSession: MediaSession? = null
    private var trackSelector: DefaultTrackSelector? = null
    private var trackFacts = TrackSupportFacts()
    private var capabilityResult: VideoCapabilityResult? = null
    private var metadataCapabilityResult: VideoCapabilityResult? = null
    private var completed = false
    private var exitReason = "back"
    private var playbackError: NativePlayerPublicError? = null
    private var retryCount = 0
    private var resumeOnForeground = false
    private var restoredPositionMs = 0L
    private var restoredPlayWhenReady: Boolean? = null
    private var restoredTrackSelection: Bundle? = null
    private var lastSafePositionMs = 0L
    private val finishing = AtomicBoolean(false)
    private val destroyed = AtomicBoolean(false)
    private val mainHandler = Handler(Looper.getMainLooper())
    private var tickerScheduled = false
    @Volatile private var latestSnapshot = NativePlaybackSnapshot()

    private val snapshotTicker = object : Runnable {
        override fun run() {
            if (destroyed.get()) return
            updateSnapshot(emitEvent = false)
            mainHandler.postDelayed(this, SNAPSHOT_INTERVAL_MS)
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
                clearRestore = false,
            )
            return
        }

        restoredPositionMs = savedInstanceState?.getLong(STATE_POSITION_MS)
            ?: session!!.args.startPositionMs
        restoredPlayWhenReady = if (savedInstanceState?.containsKey(STATE_PLAY_WHEN_READY) == true) {
            savedInstanceState.getBoolean(STATE_PLAY_WHEN_READY)
        } else null
        restoredTrackSelection = savedInstanceState?.getBundle(STATE_TRACK_SELECTION)
        lastSafePositionMs = restoredPositionMs

        configureFullscreen()
        createContentView()
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() = finishWithResult()
        })

        NativePlayerActivityRegistry.register(this)
        initializePlayer()
        scheduleTickerOnce()
    }

    private fun configureFullscreen() {
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, window.decorView).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
    }

    private fun createContentView() {
        playerView = PlayerView(this).apply {
            useController = true
            controllerAutoShow = true
            controllerHideOnTouch = true
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
        }
        rootView = FrameLayout(this).apply {
            setBackgroundColor(Color.BLACK)
            addView(playerView)
        }
        setContentView(rootView)
    }

    private fun initializePlayer() {
        if (destroyed.get() || finishing.get()) return
        val args = session?.args ?: return
        releasePlayer(rememberPosition = false)
        playbackError = null
        val root = rootView
        if (root != null) errorOverlay?.let(root::removeView)
        errorOverlay = null

        if (args.codec != null || args.width != null || args.height != null || args.frameRate != null) {
            metadataCapabilityResult = CodecCapabilityPreflight.inspect(
                VideoCapabilityMetadata(
                    sampleMimeType = args.codec?.takeIf { it.startsWith("video/") },
                    codecs = args.codec,
                    width = args.width,
                    height = args.height,
                    frameRate = args.frameRate,
                    averageBitrate = args.bitrate,
                    bitDepth = args.bitDepth,
                    hdrType = if (args.hdr == true) HdrType.UNKNOWN_HDR else HdrType.UNKNOWN,
                ),
            )
            capabilityResult = metadataCapabilityResult
        }

        val httpFactory = DefaultHttpDataSource.Factory()
            .setConnectTimeoutMs(15_000)
            .setReadTimeoutMs(30_000)
            .setAllowCrossProtocolRedirects(false)
            .setDefaultRequestProperties(mapOf("Authorization" to "Bearer ${args.authorizationToken}"))
        val renderersFactory = DefaultRenderersFactory(this).setEnableDecoderFallback(true)
        val selector = DefaultTrackSelector(this)
        restoredTrackSelection?.let { bundle ->
            try {
                selector.parameters = DefaultTrackSelector.Parameters.fromBundle(bundle)
            } catch (_: RuntimeException) {
                restoredTrackSelection = null
            }
        }
        trackSelector = selector
        val exoPlayer = ExoPlayer.Builder(this, renderersFactory)
            .setTrackSelector(selector)
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
        val mimeHint = NativePlayerMimePolicy.sanitizeHint(args.mimeType)
        if (NativePlayerMimePolicy.shouldApplyToMediaItem(mimeHint)) itemBuilder.setMimeType(mimeHint)
        exoPlayer.setMediaItem(itemBuilder.build(), restoredPositionMs.coerceAtLeast(0))
        exoPlayer.playWhenReady = restoredPlayWhenReady ?: args.autoplay
        mediaSession = MediaSession.Builder(this, exoPlayer).build()
        exoPlayer.prepare()
        updateSnapshot(emitEvent = true)
    }

    private fun cacheKey(args: OpenNativePlayerArgs): String =
        "telegram:${args.folderId ?: "home"}:${args.messageId}"

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
        val groupFacts = tracks.groups.map { group ->
            TrackGroupFacts(
                group.type,
                (0 until group.length).map(group::isTrackSupported),
                (0 until group.length).map(group::isTrackSelected),
            )
        }
        trackFacts = TrackSupportAnalyzer.analyze(groupFacts)
        val authoritativeFormat = tracks.groups
            .asSequence()
            .filter { it.type == C.TRACK_TYPE_VIDEO }
            .flatMap { group ->
                (0 until group.length).asSequence().map { index -> Triple(group, index, group.isTrackSelected(index)) }
            }
            .sortedByDescending { it.third }
            .firstOrNull { (group, index) -> group.isTrackSupported(index) }
            ?.let { (group, index) -> group.getTrackFormat(index) }
        if (authoritativeFormat != null) {
            capabilityResult = CodecCapabilityPreflight.inspect(VideoCapabilityMetadata.fromFormat(authoritativeFormat))
            val report = capabilityResult!!
            Log.i(
                TAG,
                "Video capability ${report.status}/${report.reasonCode}; codec=${report.codecFamily}; " +
                    "profile=${report.hevcProfile}; hdr=${report.hdrType}; decoders=${report.decoderCount}",
            )
        }
        if (trackFacts.primaryPlayback == PrimaryTrackPlayback.NO_PLAYABLE_PRIMARY_TRACKS && playbackError == null) {
            showFatalError(NativePlayerErrorMapper.noPlayableTracks(trackFacts, capabilityResult))
        }
    }

    override fun onVideoSizeChanged(videoSize: VideoSize) {
        updatePictureInPicture(videoSize.width, videoSize.height)
    }

    override fun onPlayerError(error: PlaybackException) {
        showFatalError(NativePlayerErrorMapper.map(error, trackFacts, capabilityResult))
    }

    private fun showFatalError(error: NativePlayerPublicError) {
        if (finishing.get() || destroyed.get()) return
        lastSafePositionMs = safePosition().takeIf { it > 0 } ?: lastSafePositionMs
        playbackError = error
        exitReason = "error"
        player?.pause()
        updateSnapshot(emitEvent = true)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N && isInPictureInPictureMode) {
            finishWithResult(error)
            return
        }
        showErrorOverlay(error)
    }

    private fun showErrorOverlay(error: NativePlayerPublicError) {
        val root = rootView ?: return
        errorOverlay?.let(root::removeView)
        val state = NativePlaybackErrorPolicy.overlay(error, retryCount)
        val panel = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            setPadding(dp(32), dp(24), dp(32), dp(24))
            setBackgroundColor(Color.rgb(24, 24, 24))
            addView(TextView(context).apply {
                text = state.title
                textSize = 22f
                setTextColor(Color.WHITE)
                gravity = Gravity.CENTER
            })
            addView(TextView(context).apply {
                text = state.message
                textSize = 16f
                setTextColor(Color.LTGRAY)
                gravity = Gravity.CENTER
                setPadding(0, dp(12), 0, dp(20))
            })
            if (state.canRetry) {
                addView(Button(context).apply {
                    text = "Retry"
                    contentDescription = "Retry playback"
                    setOnClickListener { retryPlayback() }
                })
            }
            addView(Button(context).apply {
                text = "Close"
                contentDescription = "Close native player"
                setOnClickListener { finishWithResult(error) }
            })
        }
        errorOverlay = FrameLayout(this).apply {
            setBackgroundColor(Color.argb(230, 0, 0, 0))
            isClickable = true
            isFocusable = true
            addView(panel, FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.CENTER,
            ))
        }
        root.addView(errorOverlay, FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT,
        ))
        errorOverlay?.requestFocus()
    }

    internal fun retryPlayback() {
        val error = playbackError ?: return
        if (!NativePlaybackErrorPolicy.isRetryEligible(error, retryCount)) return
        if (NativePlayerSessionStore.get(sessionId) !== session) {
            showFatalError(
                NativePlayerPublicError("authentication", "SESSION_EXPIRED", "The private playback session has expired."),
            )
            return
        }
        retryCount += 1
        restoredPositionMs = lastSafePositionMs.coerceAtLeast(0)
        restoredPlayWhenReady = true
        completed = false
        initializePlayer()
    }

    override fun onStart() {
        super.onStart()
        if (resumeOnForeground && !isInPictureInPictureMode && playbackError == null) {
            player?.play()
            resumeOnForeground = false
        }
    }

    override fun onStop() {
        if (!isInPictureInPictureMode) {
            resumeOnForeground = player?.isPlaying == true && playbackError == null
            player?.pause()
        }
        super.onStop()
    }

    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && PictureInPicturePolicy.isEligible(
                trackFacts.hasPlayableVideoTrack,
                player?.isPlaying == true,
                playbackError != null,
            )
        ) {
            try {
                enterPictureInPictureMode(currentPictureInPictureParams())
            } catch (_: IllegalArgumentException) {
                // Device-specific PiP rejection is non-fatal.
            }
        }
    }

    override fun onPictureInPictureModeChanged(inPictureInPictureMode: Boolean, newConfig: Configuration) {
        super.onPictureInPictureModeChanged(inPictureInPictureMode, newConfig)
        playerView?.useController = !inPictureInPictureMode
        if (!inPictureInPictureMode) {
            playerView?.showController()
            errorOverlay?.visibility = View.VISIBLE
        }
    }

    private fun updatePictureInPicture(width: Int, height: Int) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val ratio = PictureInPicturePolicy.sanitizeAspectRatio(width, height) ?: return
        try {
            setPictureInPictureParams(
                PictureInPictureParams.Builder()
                    .setAspectRatio(Rational(ratio.numerator, ratio.denominator))
                    .build(),
            )
        } catch (_: IllegalArgumentException) {
            // Some devices apply stricter PiP limits; playback continues normally.
        }
    }

    @RequiresApi(Build.VERSION_CODES.O)
    private fun currentPictureInPictureParams(): PictureInPictureParams {
        val size = player?.videoSize
        val ratio = size?.let { PictureInPicturePolicy.sanitizeAspectRatio(it.width, it.height) }
        return PictureInPictureParams.Builder().apply {
            ratio?.let { setAspectRatio(Rational(it.numerator, it.denominator)) }
        }.build()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString(STATE_SESSION_ID, sessionId)
        outState.putLong(STATE_POSITION_MS, safePosition())
        outState.putBoolean(STATE_PLAY_WHEN_READY, player?.playWhenReady ?: false)
        player?.trackSelectionParameters?.toBundle()?.let { outState.putBundle(STATE_TRACK_SELECTION, it) }
        super.onSaveInstanceState(outState)
    }

    fun finishFromExternal() {
        if (destroyed.get()) return
        exitReason = "external"
        finishWithResult()
    }

    fun playbackSnapshot(): NativePlaybackSnapshot = latestSnapshot

    internal fun showErrorForTest(error: NativePlayerPublicError) = showFatalError(error)
    internal fun isErrorOverlayVisibleForTest(): Boolean = errorOverlay?.visibility == View.VISIBLE
    internal fun retryCountForTest(): Int = retryCount
    internal fun hasPlayerForTest(): Boolean = player != null

    private fun scheduleTickerOnce() {
        if (tickerScheduled) return
        tickerScheduled = true
        mainHandler.post(snapshotTicker)
    }

    private fun updateSnapshot(emitEvent: Boolean) {
        val exoPlayer = player
        val position = safePosition()
        if (playbackError == null && position > 0) lastSafePositionMs = position
        val state = when {
            playbackError != null -> "error"
            exoPlayer == null -> "idle"
            exoPlayer.playbackState == Player.STATE_BUFFERING -> "buffering"
            exoPlayer.playbackState == Player.STATE_ENDED -> "ended"
            exoPlayer.isPlaying -> "playing"
            exoPlayer.playbackState == Player.STATE_READY && !exoPlayer.playWhenReady -> "paused"
            exoPlayer.playbackState == Player.STATE_READY -> "ready"
            else -> "idle"
        }
        val snapshot = NativePlaybackSnapshot(state, exoPlayer?.isPlaying == true, position, safeDuration())
        val transitioned = NativeEventTransitionPolicy.shouldEmit(
            latestSnapshot.state,
            latestSnapshot.isPlaying,
            state,
            snapshot.isPlaying,
        )
        latestSnapshot = snapshot
        if (emitEvent && transitioned) session?.stateListener?.invoke(snapshot)
    }

    private fun safePosition(): Long = player?.currentPosition
        ?.takeIf { it != C.TIME_UNSET }
        ?.coerceAtLeast(0)
        ?: lastSafePositionMs.coerceAtLeast(0)

    private fun safeDuration(): Long = player?.duration
        ?.takeIf { it != C.TIME_UNSET }
        ?.coerceAtLeast(0)
        ?: 0

    private fun finishWithResult(
        error: NativePlayerPublicError? = playbackError,
        clearRestore: Boolean = true,
    ) {
        if (!finishing.compareAndSet(false, true)) return
        if (clearRestore) PendingNativePlayerRestoreStore.clear(this)
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

    private fun releasePlayer(rememberPosition: Boolean = true) {
        val exoPlayer = player
        if (rememberPosition && exoPlayer != null) {
            val position = exoPlayer.currentPosition
            if (position != C.TIME_UNSET && position >= 0) lastSafePositionMs = position
        }
        playerView?.player = null
        mediaSession?.release()
        mediaSession = null
        exoPlayer?.removeListener(this)
        exoPlayer?.release()
        player = null
        trackSelector = null
    }

    override fun onDestroy() {
        if (destroyed.compareAndSet(false, true)) {
            tickerScheduled = false
            mainHandler.removeCallbacksAndMessages(null)
            NativePlayerActivityRegistry.clear(this)
            releasePlayer()
            errorOverlay = null
            rootView = null
            playerView = null
            if (!isChangingConfigurations) NativePlayerSessionStore.remove(sessionId)
            session = null
        }
        super.onDestroy()
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

    companion object {
        private const val TAG = "NativePlayer"
        private const val SNAPSHOT_INTERVAL_MS = 1_000L
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
        private const val STATE_TRACK_SELECTION = "nativePlayer.trackSelection"
    }
}
