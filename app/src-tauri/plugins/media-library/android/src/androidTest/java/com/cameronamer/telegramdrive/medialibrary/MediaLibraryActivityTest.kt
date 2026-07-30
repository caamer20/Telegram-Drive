package com.cameronamer.telegramdrive.medialibrary

import android.app.Activity
import android.app.Instrumentation
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.espresso.Espresso.onView
import androidx.test.espresso.action.ViewActions.click
import androidx.test.espresso.assertion.ViewAssertions.matches
import androidx.test.espresso.intent.Intents
import androidx.test.espresso.intent.matcher.IntentMatchers.hasComponent
import androidx.test.espresso.matcher.ViewMatchers.withContentDescription
import androidx.test.espresso.matcher.ViewMatchers.withText
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.cameronamer.telegramdrive.medialibrary.data.MediaType
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaDatabase
import com.cameronamer.telegramdrive.nativeplayer.NativePlayerActivity
import com.cameronamer.telegramdrive.nativeplayer.NativePlayerResultCodec
import com.cameronamer.telegramdrive.nativeplayer.NativePlayerResultData
import kotlinx.coroutines.runBlocking
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.hamcrest.CoreMatchers.containsString
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MediaLibraryActivityTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private var server: MockWebServer? = null

    @After fun cleanUp() {
        MediaLibrarySessionStore.clear()
        MediaLibraryActivityRegistry.close()
        server?.shutdown()
    }

    @Test fun activityIsRegisteredAndNotExported() {
        val info = context.packageManager.getActivityInfo(
            ComponentName(context, MediaLibraryActivity::class.java),
            0,
        )
        assertFalse(info.exported)
    }

    @Test fun offlineStartupAndProcessRecreationWithoutTokenDoNotCrash() {
        val scenario = ActivityScenario.launch<MediaLibraryActivity>(
            Intent(context, MediaLibraryActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
        onView(withText(containsString("No offline media"))).check(matches(withText(containsString("No offline media"))))
        MediaLibrarySessionStore.clear()
        scenario.recreate()
        onView(withText(containsString("Offline"))).check(matches(withText(containsString("Offline"))))
        scenario.close()
    }

    @Test fun existingRoomDataRendersFilterAndPreviewOpen() {
        val account = 556677L
        runBlocking {
            val database = TelegramMediaDatabase.get(context)
            database.withTransactionClearAccount(account)
            database.mediaDao().upsertMedia(listOf(MediaLibraryPureInstrumentedFixtures.item(account)))
        }
        val scenario = launchOnline(account)
        onView(withText("instrumented-photo.jpg")).check(matches(withText("instrumented-photo.jpg"))).perform(click())
        onView(withText("instrumented-photo.jpg")).check(matches(withText("instrumented-photo.jpg")))
        androidx.test.espresso.Espresso.pressBack()
        onView(withContentDescription("Filter media")).perform(click())
        onView(withText("Filter media")).check(matches(withText("Filter media")))
        scenario.close()
    }

    @Test fun videoPlayDelegatesToSharedNativePlayerActivityAndNoPlayerIsOwnedByLibrary() {
        val account = 667788L
        runBlocking {
            val database = TelegramMediaDatabase.get(context)
            database.withTransactionClearAccount(account)
            database.mediaDao().upsertMedia(listOf(MediaLibraryPureInstrumentedFixtures.item(account, MediaType.VIDEO)))
        }
        Intents.init()
        try {
            val scenario = launchOnline(account)
            Intents.intending(hasComponent(NativePlayerActivity::class.java.name)).respondWith(
                Instrumentation.ActivityResult(
                    Activity.RESULT_OK,
                    NativePlayerResultCodec.toIntent(
                        NativePlayerResultData(positionMs = 1_000, durationMs = 20_000, completed = false),
                    ),
                ),
            )
            onView(withText("instrumented-video.mp4")).perform(click())
            onView(withText("Play in native player")).perform(click())
            Intents.intended(hasComponent(NativePlayerActivity::class.java.name))
            onView(withText("instrumented-video.mp4")).check(matches(withText("instrumented-video.mp4")))
            onView(withText("Play in native player")).check(matches(withText("Play in native player")))
            assertTrue(MediaLibraryActivity::class.java.declaredFields.none { it.type.name.contains("ExoPlayer") })
            scenario.close()
        } finally {
            Intents.release()
        }
    }

    private fun launchOnline(accountId: Long): ActivityScenario<MediaLibraryActivity> {
        val mock = MockWebServer().also { server = it; it.start() }
        mock.enqueue(MockResponse().setHeader("Content-Type", "application/json").setBody("{\"accountId\":$accountId,\"displayName\":\"Test\"}"))
        mock.enqueue(MockResponse().setHeader("Content-Type", "application/json").setBody("{\"items\":[]}"))
        val args = OpenMediaLibraryArgs().apply {
            baseUrl = mock.url("/").toString().removeSuffix("/").replace("localhost", "127.0.0.1")
            authorizationToken = "instrumentation-only-token"
        }
        val session = MediaLibrarySessionStore.create(args).apply { this.accountId = accountId }
        return ActivityScenario.launch(
            Intent(context, MediaLibraryActivity::class.java)
                .putExtra(MediaLibraryActivity.EXTRA_SESSION_ID, session.id)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }
}

private object MediaLibraryPureInstrumentedFixtures {
    fun item(account: Long, type: MediaType = MediaType.IMAGE) =
        com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaEntity(
            account, 99, if (type == MediaType.VIDEO) 9 else 8, 99, "Test folder", null,
            if (type == MediaType.VIDEO) "instrumented-video.mp4" else "instrumented-photo.jpg",
            if (type == MediaType.VIDEO) "instrumented-video.mp4" else "instrumented-photo.jpg",
            if (type == MediaType.VIDEO) "instrumented-video.mp4" else "instrumented-photo.jpg",
            if (type == MediaType.VIDEO) "instrumented-video.mp4 test folder" else "instrumented-photo.jpg test folder",
            null, type, if (type == MediaType.VIDEO) "video/mp4" else "image/jpeg",
            if (type == MediaType.VIDEO) "mp4" else "jpg", 100, 1_700_000_000,
            if (type == MediaType.VIDEO) 20 else null, 1920, 1080, false, null, null,
            com.cameronamer.telegramdrive.medialibrary.data.ThumbnailStatus.NO_THUMBNAIL, 1, false,
        )
}
