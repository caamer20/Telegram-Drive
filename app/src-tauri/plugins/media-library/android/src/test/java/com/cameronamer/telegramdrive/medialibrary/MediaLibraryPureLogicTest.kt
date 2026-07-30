package com.cameronamer.telegramdrive.medialibrary

import androidx.sqlite.db.SupportSQLiteProgram
import com.cameronamer.telegramdrive.medialibrary.data.MediaFilter
import com.cameronamer.telegramdrive.medialibrary.data.MediaQueryBuilder
import com.cameronamer.telegramdrive.medialibrary.data.MediaScope
import com.cameronamer.telegramdrive.medialibrary.data.MediaSort
import com.cameronamer.telegramdrive.medialibrary.data.ResolutionFilter
import com.cameronamer.telegramdrive.medialibrary.data.SearchNormalizer
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaEntity
import com.cameronamer.telegramdrive.medialibrary.data.ThumbnailFilter
import com.cameronamer.telegramdrive.medialibrary.data.ThumbnailStatus
import com.cameronamer.telegramdrive.medialibrary.data.MediaType
import com.cameronamer.telegramdrive.medialibrary.network.MediaLibraryApi
import kotlinx.coroutines.test.runTest
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class MediaLibraryPureLogicTest {
    @Test fun `search normalization folds case spaces and diacritics`() {
        assertEquals("resume photo.jpg", SearchNormalizer.normalize("  Résumé   PHOTO.JPG "))
        assertEquals("a\\%b\\_c\\\\d", SearchNormalizer.escapeLike("a%b_c\\d"))
    }

    @Test fun `all sorts are whitelisted and deterministic`() {
        MediaSort.entries.forEach { sort ->
            val query = MediaQueryBuilder.build(7, "", MediaFilter(), sort)
            assertTrue(query.sql.startsWith("SELECT * FROM telegram_media WHERE accountId = ? AND deleted = 0"))
            assertTrue(query.sql.contains("peerId"))
            assertTrue(query.sql.contains("messageId"))
            assertFalse(query.sql.contains("nulls", ignoreCase = true))
        }
    }

    @Test fun `combined filters bind values and never concatenate input`() {
        val hostile = "jpg' OR 1=1 --"
        val query = MediaQueryBuilder.build(
            9,
            "100%_safe",
            MediaFilter(
                scope = MediaScope.VIDEOS,
                peerId = 44,
                dateFromEpochSeconds = 10,
                dateToEpochSeconds = 20,
                minimumSizeBytes = 30,
                maximumSizeBytes = 40,
                minimumDurationSeconds = 2,
                maximumDurationSeconds = 8,
                extension = hostile,
                mimeType = "video/mp4; codec=x",
                thumbnail = ThumbnailFilter.HAS_THUMBNAIL,
                resolution = ResolutionFilter.FULL_HD,
            ),
            MediaSort.LARGEST,
        )
        val bindings = BoundArguments()
        query.bindTo(bindings)
        assertFalse(query.sql.contains(hostile))
        assertEquals(9L, bindings.values[1])
        assertTrue(bindings.values.values.contains("%100\\%\\_safe%"))
        assertTrue(bindings.values.values.contains(hostile.lowercase()))
        assertTrue(bindings.values.values.contains("video/mp4"))
    }

    @Test fun `compound identity is account peer message`() {
        val first = entity(1, 2, 3)
        val second = entity(2, 2, 3)
        assertEquals("1_2_3", first.stableKey)
        assertFalse(first.stableKey == second.stableKey)
    }

    @Test fun `api parses safe dto and maps to entity`() = runTest {
        MockWebServer().use { server ->
            server.enqueue(MockResponse().setBody("""{
              "items":[{"accountId":7,"peerId":11,"folderId":11,"messageId":22,
              "peerName":"Photos","senderId":8,"dateEpochSeconds":100,
              "displayName":"picture.jpg","originalFilename":"picture.jpg","caption":"caption",
              "mediaType":"image","mimeType":"image/jpeg","extension":"jpg","sizeBytes":55,
              "durationSeconds":null,"width":640,"height":480,"thumbnailAvailable":true,"thumbnailVariant":"m"}],
              "nextOffsetMessageId":21,"hasMore":true,"messagesScanned":3,"mediaFound":1,
              "newestScannedMessageId":22,"oldestScannedMessageId":20,"reachedNewerThanBoundary":false}
            """.trimIndent()).setHeader("Content-Type", "application/json"))
            val baseUrl = server.url("/").toString().removeSuffix("/").replace("localhost", "127.0.0.1")
            val api = MediaLibraryApi(baseUrl, "secret-token")
            val page = api.mediaPage(11, 0)
            val mapped = page.items.single().toEntity(200)
            assertEquals(MediaType.IMAGE, mapped.mediaType)
            assertEquals("picture.jpg", mapped.originalFilename)
            assertEquals("caption", mapped.caption)
            assertEquals(55L, mapped.sizeBytes)
            val request = server.takeRequest()
            assertEquals("Bearer secret-token", request.getHeader("Authorization"))
            assertFalse(request.path.orEmpty().contains("token"))
            assertFalse(api.serializableStateForTest().contains("secret-token"))
        }
    }

    private class BoundArguments : SupportSQLiteProgram {
        val values = linkedMapOf<Int, Any?>()
        override fun bindNull(index: Int) { values[index] = null }
        override fun bindLong(index: Int, value: Long) { values[index] = value }
        override fun bindDouble(index: Int, value: Double) { values[index] = value }
        override fun bindString(index: Int, value: String) { values[index] = value }
        override fun bindBlob(index: Int, value: ByteArray) { values[index] = value }
        override fun clearBindings() = values.clear()
        override fun close() = Unit
    }

    companion object {
        fun entity(account: Long, peer: Long, message: Int, type: MediaType = MediaType.IMAGE) =
            TelegramMediaEntity(
                account, peer, message, peer, "Folder $peer", null,
                "item-$message.jpg", "item-$message.jpg", "item-$message.jpg", "item-$message.jpg folder $peer",
                null, type, if (type == MediaType.VIDEO) "video/mp4" else "image/jpeg",
                if (type == MediaType.VIDEO) "mp4" else "jpg", message * 100L, message * 10L,
                if (type == MediaType.VIDEO) message else null, 1920, 1080, true, "m", null,
                ThumbnailStatus.NOT_REQUESTED, 1, false,
            )
    }
}
