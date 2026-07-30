package com.cameronamer.telegramdrive.medialibrary

import android.content.Context
import androidx.paging.PagingSource
import androidx.room.Room
import androidx.test.core.app.ApplicationProvider
import com.cameronamer.telegramdrive.medialibrary.data.MediaFilter
import com.cameronamer.telegramdrive.medialibrary.data.MediaPlaybackStateEntity
import com.cameronamer.telegramdrive.medialibrary.data.MediaQueryBuilder
import com.cameronamer.telegramdrive.medialibrary.data.MediaScope
import com.cameronamer.telegramdrive.medialibrary.data.MediaSort
import com.cameronamer.telegramdrive.medialibrary.data.MediaSyncStateEntity
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaDatabase
import com.cameronamer.telegramdrive.medialibrary.data.ThumbnailFilter
import com.cameronamer.telegramdrive.medialibrary.data.MediaType
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class MediaLibraryRoomTest {
    private lateinit var database: TelegramMediaDatabase

    @Before fun setUp() {
        database = Room.inMemoryDatabaseBuilder(
            ApplicationProvider.getApplicationContext<Context>(),
            TelegramMediaDatabase::class.java,
        ).allowMainThreadQueries().build()
    }

    @After fun tearDown() = database.close()

    @Test fun `compound keys isolate accounts peers and duplicate upsert`() = runTest {
        val dao = database.mediaDao()
        dao.upsertMedia(listOf(
            MediaLibraryPureLogicTest.entity(1, 10, 5),
            MediaLibraryPureLogicTest.entity(2, 10, 5),
            MediaLibraryPureLogicTest.entity(1, 11, 5),
        ))
        dao.upsertMedia(listOf(MediaLibraryPureLogicTest.entity(1, 10, 5).copy(displayName = "updated")))
        assertEquals("updated", dao.media(1, 10, 5)?.displayName)
        assertEquals(2, dao.mediaCount(1))
        assertEquals(1, dao.mediaCount(2))
    }

    @Test fun `filters search sorts and deleted exclusion work through paging`() = runTest {
        val dao = database.mediaDao()
        dao.upsertMedia(listOf(
            MediaLibraryPureLogicTest.entity(1, 10, 1).copy(displayName = "Zebra", normalizedName = "zebra", normalizedSearchText = "zebra beach", sizeBytes = null),
            MediaLibraryPureLogicTest.entity(1, 11, 2, MediaType.VIDEO).copy(displayName = "Alpha", normalizedName = "alpha", normalizedSearchText = "alpha movies", durationSeconds = 99, thumbnailAvailable = false),
            MediaLibraryPureLogicTest.entity(1, 10, 3).copy(displayName = "Hidden", normalizedName = "hidden", normalizedSearchText = "hidden", deleted = true),
        ))
        assertEquals(listOf("Alpha"), load(1, "alpha", MediaFilter(), MediaSort.NAME_ASC).map { it.displayName })
        assertEquals(listOf("Alpha"), load(1, "", MediaFilter(scope = MediaScope.VIDEOS), MediaSort.NEWEST).map { it.displayName })
        assertEquals(listOf("Zebra"), load(1, "", MediaFilter(peerId = 10), MediaSort.NAME_ASC).map { it.displayName })
        assertEquals(listOf("Alpha"), load(1, "", MediaFilter(minimumDurationSeconds = 50), MediaSort.LONGEST_VIDEO).map { it.displayName })
        assertEquals(listOf("Alpha"), load(1, "", MediaFilter(dateFromEpochSeconds = 15), MediaSort.OLDEST).map { it.displayName })
        assertEquals(listOf("Zebra"), load(1, "", MediaFilter(thumbnail = ThumbnailFilter.HAS_THUMBNAIL), MediaSort.NEWEST).map { it.displayName })
        assertEquals(listOf("Alpha"), load(1, "", MediaFilter(maximumSizeBytes = 250), MediaSort.NEWEST).map { it.displayName })
        assertEquals(listOf("Alpha"), load(1, "", MediaFilter(extension = "mp4", mimeType = "video/mp4", thumbnail = ThumbnailFilter.NO_THUMBNAIL), MediaSort.NAME_ASC).map { it.displayName })
        assertEquals(listOf("Zebra", "Alpha"), load(1, "", MediaFilter(), MediaSort.NAME_DESC).map { it.displayName })
        assertEquals(listOf("Alpha", "Zebra"), load(1, "", MediaFilter(), MediaSort.LARGEST).map { it.displayName })
        assertEquals(listOf("Zebra", "Alpha"), load(1, "", MediaFilter(), MediaSort.FOLDER_ASC).map { it.displayName })
    }

    @Test fun `paging source invalidates after transactional upsert`() = runBlocking {
        val source = database.mediaDao().pagingSource(MediaQueryBuilder.build(1, "", MediaFilter(), MediaSort.NEWEST))
        source.load(PagingSource.LoadParams.Refresh(null, 10, false))
        var invalidated = false
        source.registerInvalidatedCallback { invalidated = true }
        database.mediaDao().upsertMedia(listOf(MediaLibraryPureLogicTest.entity(1, 2, 3)))
        database.invalidationTracker.refreshVersionsAsync()
        withTimeout(2_000) {
            while (!invalidated && !source.invalid) delay(10)
        }
        assertTrue(invalidated || source.invalid)
    }

    @Test fun `playback and sync cursors persist and clear by account`() = runTest {
        val dao = database.mediaDao()
        dao.upsertPlaybackState(MediaPlaybackStateEntity(1, 9, 4, 500, 1000, false, 20))
        dao.upsertPlaybackState(MediaPlaybackStateEntity(2, 9, 4, 700, null, false, 21))
        dao.upsertSyncState(MediaSyncStateEntity(1, 9, 3, 10, 4, false, 1, null, null))
        assertEquals(500L, dao.playbackState(1, 9, 4)?.positionMs)
        assertEquals(3, dao.syncState(1, 9)?.nextOffsetMessageId)
        database.withTransactionClearAccount(1)
        assertNull(dao.playbackState(1, 9, 4))
        assertNull(dao.syncState(1, 9))
        assertEquals(700L, dao.playbackState(2, 9, 4)?.positionMs)
    }

    private suspend fun load(account: Long, search: String, filter: MediaFilter, sort: MediaSort) =
        when (val result = database.mediaDao().pagingSource(MediaQueryBuilder.build(account, search, filter, sort)).load(
            PagingSource.LoadParams.Refresh(null, 50, false),
        )) {
            is PagingSource.LoadResult.Page -> result.data
            is PagingSource.LoadResult.Error -> throw result.throwable
            is PagingSource.LoadResult.Invalid -> error("invalid paging source")
        }
}
