package com.cameronamer.telegramdrive.nativeplayer

import android.content.Context
import android.content.Intent
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.espresso.Espresso.onView
import androidx.test.espresso.action.ViewActions.click
import androidx.test.espresso.matcher.ViewMatchers.withContentDescription
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class NativePlayerActivityTest {
    private fun launch(): ActivityScenario<NativePlayerActivity> {
        val args = OpenNativePlayerArgs().apply {
            messageId = 1
            title = "Test"
            fileName = "test.mp4"
            mimeType = "video/mp4"
            autoplay = false
            streamUrl = "http://127.0.0.1:65534/stream/home/1"
            authorizationToken = "instrumentation-only"
        }
        val session = NativePlayerSessionStore.create(args)
        val context = ApplicationProvider.getApplicationContext<Context>()
        return ActivityScenario.launch(
            Intent(context, NativePlayerActivity::class.java)
                .putExtra(NativePlayerActivity.EXTRA_SESSION_ID, session.id),
        )
    }

    @Test
    fun opensRecreatesAndClosesWithoutLeakingRegistry() {
        val scenario = launch()
        scenario.recreate()
        scenario.onActivity { it.finishFromExternal() }
        scenario.close()
        assertEquals("idle", NativePlayerActivityRegistry.snapshot().state)
    }

    @Test
    fun repeatedOpenAndBackReturnCleanly() {
        repeat(2) {
            val scenario = launch()
            scenario.onActivity { it.onBackPressedDispatcher.onBackPressed() }
            scenario.close()
        }
        assertEquals("idle", NativePlayerActivityRegistry.snapshot().state)
    }

    @Test
    fun processRecoveryPersistsIdentityButNeverStreamSecrets() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        PendingNativePlayerRestoreStore.clear(context)
        val identity = Intent()
            .putExtra(NativePlayerActivity.EXTRA_FOLDER_ID, 9L)
            .putExtra(NativePlayerActivity.EXTRA_MESSAGE_ID, 12)
            .putExtra(NativePlayerActivity.EXTRA_TITLE, "Movie")
            .putExtra(NativePlayerActivity.EXTRA_FILE_NAME, "movie.mkv")
            .putExtra(NativePlayerActivity.EXTRA_AUTOPLAY, true)
        PendingNativePlayerRestoreStore.save(context, identity, 3456, false)

        val restored = PendingNativePlayerRestoreStore.take(context)!!
        assertEquals(9L, restored.folderId)
        assertEquals(12, restored.messageId)
        assertEquals(3456, restored.startPositionMs)
        assertFalse(restored.autoplay)
        val serialized = restored.toJsObject().toString().lowercase()
        assertFalse(serialized.contains("token"))
        assertFalse(serialized.contains("url"))
        assertNull(PendingNativePlayerRestoreStore.take(context))
    }

    @Test
    fun restoreExpiresIsOneTimeAndMalformedDataIsDiscarded() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        PendingNativePlayerRestoreStore.clear(context)
        val identity = Intent()
            .putExtra(NativePlayerActivity.EXTRA_MESSAGE_ID, 12)
            .putExtra(NativePlayerActivity.EXTRA_TITLE, "Movie")
        PendingNativePlayerRestoreStore.save(context, identity, 5, true, nowMs = 1_000)
        assertNull(PendingNativePlayerRestoreStore.take(
            context,
            nowMs = 1_000 + PendingNativePlayerRestoreStore.RESTORE_TTL_MS + 1,
        ))
        assertNull(PendingNativePlayerRestoreStore.take(context))

        PendingNativePlayerRestoreStore.save(
            context,
            Intent().putExtra(NativePlayerActivity.EXTRA_MESSAGE_ID, 0),
            0,
            true,
        )
        assertNull(PendingNativePlayerRestoreStore.take(context))
    }

    @Test
    fun injectedFatalErrorShowsRetryAndCloseOverlay() {
        val scenario = launch()
        scenario.onActivity {
            it.showErrorForTest(
                NativePlayerPublicError("server", "HTTP_503", "The local stream is temporarily unavailable."),
            )
            assertTrue(it.isErrorOverlayVisibleForTest())
        }
        onView(withContentDescription("Retry playback")).perform(click())
        scenario.onActivity {
            assertEquals(1, it.retryCountForTest())
            assertTrue(it.hasPlayerForTest())
            it.showErrorForTest(
                NativePlayerPublicError("container", "CORRUPT_MEDIA", "The media is corrupt."),
            )
        }
        onView(withContentDescription("Close native player")).perform(click())
        scenario.close()
        assertEquals("idle", NativePlayerActivityRegistry.snapshot().state)
    }

    @Test
    fun resultCodecContainsOnlyPublicSafeFields() {
        val intent = NativePlayerResultCodec.toIntent(
            NativePlayerResultData(
                positionMs = 10,
                durationMs = 20,
                exitReason = "error",
                error = NativePlayerPublicError("network", "READ_TIMEOUT", "Safe message"),
            ),
        )
        val keys = intent.extras!!.keySet().map(String::lowercase)
        assertFalse(keys.any { it.contains("token") || it.contains("authorization") || it.contains("url") || it.contains("path") })
        assertEquals("network", NativePlayerResultCodec.fromIntent(intent).error?.category)
    }
}
