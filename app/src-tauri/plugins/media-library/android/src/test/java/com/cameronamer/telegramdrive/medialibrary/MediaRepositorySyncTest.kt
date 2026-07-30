package com.cameronamer.telegramdrive.medialibrary

import android.content.Context
import androidx.room.Room
import androidx.test.core.app.ApplicationProvider
import com.cameronamer.telegramdrive.medialibrary.data.MediaSyncStateEntity
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaDatabase
import com.cameronamer.telegramdrive.medialibrary.network.MediaLibraryApi
import com.cameronamer.telegramdrive.medialibrary.repository.MediaRepository
import com.cameronamer.telegramdrive.medialibrary.repository.ThumbnailCache
import com.cameronamer.telegramdrive.medialibrary.repository.ThumbnailRepository
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.SocketPolicy
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class MediaRepositorySyncTest {
    private lateinit var database: TelegramMediaDatabase
    private lateinit var server: MockWebServer
    private lateinit var repository: MediaRepository

    @Before fun setUp() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        database = Room.inMemoryDatabaseBuilder(context, TelegramMediaDatabase::class.java).allowMainThreadQueries().build()
        server = MockWebServer().apply { start() }
        val base = server.url("/").toString().removeSuffix("/").replace("localhost", "127.0.0.1")
        val session = MediaLibrarySession("test", base, "secret", 7)
        val api = MediaLibraryApi(base, "secret")
        repository = MediaRepository(
            database, session, api,
            ThumbnailRepository(api, database.mediaDao(), ThumbnailCache(context, database.mediaDao())),
        )
    }

    @After fun tearDown() {
        repository.close()
        database.close()
        server.shutdown()
    }

    @Test fun `full sync commits every page advances cursor and completes`() = runBlocking {
        enqueuePeers()
        server.enqueue(json(page(record(10), 9, true)))
        server.enqueue(json(page(record(8), null, false)))
        repository.synchronize(7)
        await { database.mediaDao().syncState(7, 99)?.fullSyncCompleted == true }
        val state = database.mediaDao().syncState(7, 99)!!
        assertEquals(10, state.newestIndexedMessageId)
        assertEquals(8, state.oldestIndexedMessageId)
        assertEquals(null, state.nextOffsetMessageId)
        assertEquals(2, database.mediaDao().mediaCount(7))
        assertEquals(2, repository.progress.value.mediaIndexed)
    }

    @Test fun `failed page resumes from last committed cursor`() = runBlocking {
        enqueuePeers()
        server.enqueue(json(page(record(10), 9, true)))
        server.enqueue(MockResponse().setResponseCode(500))
        repository.synchronize(7)
        await { database.mediaDao().syncState(7, 99)?.lastError != null }
        assertEquals(9, database.mediaDao().syncState(7, 99)?.nextOffsetMessageId)

        enqueuePeers()
        server.enqueue(json(page(record(8), null, false)))
        repository.retry(7)
        await { database.mediaDao().syncState(7, 99)?.fullSyncCompleted == true }
        assertEquals(2, database.mediaDao().mediaCount(7))
        val mediaRequests = (0 until server.requestCount).map { server.takeRequest() }
            .filter { it.path?.contains("media-page") == true }
        assertTrue(mediaRequests.last().body.readUtf8().contains("\"offsetMessageId\":9"))
    }

    @Test fun `incremental refresh stops at boundary and updates recent metadata`() = runBlocking {
        database.mediaDao().upsertSyncState(
            MediaSyncStateEntity(7, 99, null, 100, 2, true, 1, 2, null),
        )
        database.mediaDao().upsertMedia(listOf(MediaLibraryPureLogicTest.entity(7, 99, 100)))
        enqueuePeers()
        server.enqueue(json(page(record(101), null, false, reached = true)))
        server.enqueue(json(page(record(101, caption = "edited"), null, false)))
        repository.synchronize(7)
        await { database.mediaDao().syncState(7, 99)?.newestIndexedMessageId == 101 }
        await { database.mediaDao().media(7, 99, 101)?.caption == "edited" }
        assertEquals(2, database.mediaDao().mediaCount(7))
        assertFalse(repository.progress.value.fullSyncRunning)
    }

    @Test fun `cancellation prevents subsequent pages and preserves committed data`() = runBlocking {
        enqueuePeers()
        server.enqueue(json(page(record(10), 9, true)))
        server.enqueue(MockResponse().setSocketPolicy(SocketPolicy.NO_RESPONSE))
        repository.synchronize(7)
        await { database.mediaDao().mediaCount(7) == 1 }
        repository.cancelSynchronization()
        await { !repository.progress.value.cancellationAvailable }
        assertEquals(1, database.mediaDao().mediaCount(7))
        assertEquals(9, database.mediaDao().syncState(7, 99)?.nextOffsetMessageId)
    }

    @Test fun `account change during page sync cannot cross account boundary`() = runBlocking {
        enqueuePeers()
        server.enqueue(json(page(record(10).replace("\"accountId\":7", "\"accountId\":8"), null, false)))
        repository.synchronize(7)
        await { database.mediaDao().syncState(7, 99)?.lastError != null }
        assertEquals(0, database.mediaDao().mediaCount(7))
        assertEquals(0, database.mediaDao().mediaCount(8))
    }

    @Test fun `full resync intent survives failure before first page`() = runBlocking {
        database.mediaDao().upsertSyncState(
            MediaSyncStateEntity(7, 99, null, 100, 1, true, 1, 10, null),
        )
        enqueuePeers()
        server.enqueue(MockResponse().setResponseCode(500))
        repository.fullResync(7)
        await { database.mediaDao().syncState(7, 99)?.lastError != null }
        assertFalse(database.mediaDao().syncState(7, 99)!!.fullSyncCompleted)

        enqueuePeers()
        server.enqueue(json(page(record(101), null, false)))
        repository.retry(7)
        await { database.mediaDao().syncState(7, 99)?.fullSyncCompleted == true }
        assertEquals(101, database.mediaDao().syncState(7, 99)?.newestIndexedMessageId)
    }

    private fun enqueuePeers() = server.enqueue(json("""{"items":[{"peerId":99,"folderId":99,"name":"Photos","kind":"channel"}]}"""))

    private fun record(messageId: Int, caption: String = "caption") = """{
      "accountId":7,"peerId":99,"folderId":99,"messageId":$messageId,"peerName":"Photos",
      "senderId":null,"dateEpochSeconds":$messageId,"displayName":"$messageId.jpg",
      "originalFilename":"$messageId.jpg","caption":"$caption","mediaType":"image",
      "mimeType":"image/jpeg","extension":"jpg","sizeBytes":100,"durationSeconds":null,
      "width":100,"height":100,"thumbnailAvailable":true,"thumbnailVariant":"m"}
    """.trimIndent()

    private fun page(record: String, next: Int?, more: Boolean, reached: Boolean = false) = """{
      "items":[$record],"nextOffsetMessageId":${next ?: "null"},"hasMore":$more,
      "messagesScanned":1,"mediaFound":1,"newestScannedMessageId":10,
      "oldestScannedMessageId":1,"reachedNewerThanBoundary":$reached}
    """.trimIndent()

    private fun json(body: String) = MockResponse().setHeader("Content-Type", "application/json").setBody(body)

    private suspend fun await(predicate: suspend () -> Boolean) = withTimeout(5_000) {
        while (!predicate()) delay(20)
    }
}
