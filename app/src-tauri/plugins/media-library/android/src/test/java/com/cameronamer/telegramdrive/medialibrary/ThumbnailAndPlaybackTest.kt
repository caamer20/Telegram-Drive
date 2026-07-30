package com.cameronamer.telegramdrive.medialibrary

import android.content.Context
import androidx.room.Room
import androidx.test.core.app.ApplicationProvider
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaDatabase
import com.cameronamer.telegramdrive.medialibrary.data.MediaType
import com.cameronamer.telegramdrive.medialibrary.repository.MediaRepository
import com.cameronamer.telegramdrive.medialibrary.repository.ThumbnailCache
import com.cameronamer.telegramdrive.medialibrary.repository.ThumbnailRepository
import com.cameronamer.telegramdrive.nativeplayer.NativePlayerResultData
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class ThumbnailAndPlaybackTest {
    private lateinit var database: TelegramMediaDatabase
    private lateinit var repository: MediaRepository
    private lateinit var cache: ThumbnailCache

    @Before fun setUp() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        context.filesDir.resolve("media-thumbnails").deleteRecursively()
        database = Room.inMemoryDatabaseBuilder(context, TelegramMediaDatabase::class.java).allowMainThreadQueries().build()
        cache = ThumbnailCache(context, database.mediaDao(), maximumBytes = 12)
        repository = MediaRepository(
            database,
            null,
            null,
            ThumbnailRepository(null, database.mediaDao(), cache),
        )
    }

    @After fun tearDown() = database.close()

    @Test fun `thumbnail keys are sanitized cache writes atomic and eviction bounded`() = runTest {
        val first = MediaLibraryPureLogicTest.entity(7, 8, 9).copy(thumbnailVariant = "../evil")
        val second = MediaLibraryPureLogicTest.entity(7, 8, 10).copy(thumbnailVariant = "m")
        val firstFile = cache.writeAtomic(first, ByteArray(8) { 1 })
        assertTrue(
            "path=${firstFile.absolutePath} exists=${firstFile.exists()} file=${firstFile.isFile} length=${firstFile.length()}",
            cache.validate(firstFile),
        )
        assertTrue(firstFile.name.matches(Regex("8_9_[a-z0-9_\\-]+\\.jpg")))
        cache.retain(firstFile)
        val secondFile = cache.writeAtomic(second, ByteArray(8) { 2 })
        assertTrue(cache.validate(secondFile))
        assertTrue(firstFile.exists())
        cache.release(firstFile)
        cache.evictIfNeeded(secondFile)
        assertFalse(firstFile.exists())
        cache.retain(secondFile)
        cache.clearAccount(7)
        assertTrue(secondFile.exists())
        cache.release(secondFile)
        assertFalse(secondFile.exists())
    }

    @Test fun `player request maps metadata and partial playback persists`() = runTest {
        val item = MediaLibraryPureLogicTest.entity(1, 2, 3, MediaType.VIDEO).copy(
            folderId = 44,
            displayName = "Title",
            originalFilename = "clip.mkv",
            mimeType = "video/x-matroska",
        )
        database.mediaDao().upsertMedia(listOf(item))
        val initial = repository.playerRequest(item)
        assertEquals(44L, initial.folderId)
        assertEquals("video/x-matroska", initial.mimeType)
        repository.savePlaybackResult(item, NativePlayerResultData(positionMs = 321, durationMs = 900, completed = false))
        assertEquals(321L, repository.playerRequest(item).startPositionMs)
        repository.savePlaybackResult(item, NativePlayerResultData(positionMs = 900, durationMs = 900, completed = true))
        assertEquals(0L, repository.playerRequest(item).startPositionMs)
        assertNull(database.mediaDao().playbackState(1, 2, 3))
    }

    @Test fun `offline state retains account and disables network`() = runTest {
        assertFalse(repository.online.value)
        assertNull(repository.connectAccount())
        val item = MediaLibraryPureLogicTest.entity(1, 2, 4)
        assertNull(repository.thumbnailRepository.ensureThumbnail(item))
    }

    @Test fun `thumbnail validation accepts supported images and rejects HTML`() {
        assertTrue(ThumbnailRepository.isSupportedThumbnailPayload("image/jpeg", byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 1)))
        assertFalse(ThumbnailRepository.isSupportedThumbnailPayload("text/html", "<html>".toByteArray()))
        assertFalse(ThumbnailRepository.isSupportedThumbnailPayload("image/jpeg", "not an image".toByteArray()))
    }
}
