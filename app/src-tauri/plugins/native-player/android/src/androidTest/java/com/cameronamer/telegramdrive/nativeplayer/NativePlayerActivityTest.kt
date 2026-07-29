package com.cameronamer.telegramdrive.nativeplayer

import android.content.Context
import android.content.Intent
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
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
}
