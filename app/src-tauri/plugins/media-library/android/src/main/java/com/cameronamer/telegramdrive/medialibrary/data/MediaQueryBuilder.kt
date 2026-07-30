package com.cameronamer.telegramdrive.medialibrary.data

import androidx.sqlite.db.SimpleSQLiteQuery
import androidx.sqlite.db.SupportSQLiteQuery
import java.text.Normalizer
import java.util.Locale

enum class MediaScope { ALL, IMAGES, VIDEOS }
enum class ThumbnailFilter { ANY, HAS_THUMBNAIL, NO_THUMBNAIL }
enum class ResolutionFilter { ANY, SMALL, HD, FULL_HD, FOUR_K }
enum class MediaSort {
    NEWEST, OLDEST, NAME_ASC, NAME_DESC, LARGEST, SMALLEST,
    LONGEST_VIDEO, SHORTEST_VIDEO, FOLDER_ASC, FOLDER_DESC,
}

data class MediaFilter(
    val scope: MediaScope = MediaScope.ALL,
    val peerId: Long? = null,
    val dateFromEpochSeconds: Long? = null,
    val dateToEpochSeconds: Long? = null,
    val minimumSizeBytes: Long? = null,
    val maximumSizeBytes: Long? = null,
    val minimumDurationSeconds: Int? = null,
    val maximumDurationSeconds: Int? = null,
    val extension: String? = null,
    val mimeType: String? = null,
    val thumbnail: ThumbnailFilter = ThumbnailFilter.ANY,
    val resolution: ResolutionFilter = ResolutionFilter.ANY,
)

object SearchNormalizer {
    fun normalize(value: String): String = Normalizer.normalize(value, Normalizer.Form.NFD)
        .replace(Regex("\\p{M}+"), "")
        .lowercase(Locale.ROOT)
        .trim()
        .replace(Regex("\\s+"), " ")

    fun escapeLike(value: String): String = value
        .replace("\\", "\\\\")
        .replace("%", "\\%")
        .replace("_", "\\_")
}

object MediaQueryBuilder {
    private val sortClauses = mapOf(
        MediaSort.NEWEST to "dateEpochSeconds DESC, peerId ASC, messageId DESC",
        MediaSort.OLDEST to "dateEpochSeconds ASC, peerId ASC, messageId ASC",
        MediaSort.NAME_ASC to "normalizedName ASC, peerId ASC, messageId ASC",
        MediaSort.NAME_DESC to "normalizedName DESC, peerId ASC, messageId ASC",
        MediaSort.LARGEST to "(sizeBytes IS NULL) ASC, sizeBytes DESC, dateEpochSeconds DESC, peerId ASC, messageId DESC",
        MediaSort.SMALLEST to "(sizeBytes IS NULL) ASC, sizeBytes ASC, dateEpochSeconds DESC, peerId ASC, messageId DESC",
        MediaSort.LONGEST_VIDEO to "(durationSeconds IS NULL) ASC, durationSeconds DESC, dateEpochSeconds DESC, peerId ASC, messageId DESC",
        MediaSort.SHORTEST_VIDEO to "(durationSeconds IS NULL) ASC, durationSeconds ASC, dateEpochSeconds DESC, peerId ASC, messageId DESC",
        MediaSort.FOLDER_ASC to "(peerName IS NULL) ASC, peerName COLLATE NOCASE ASC, dateEpochSeconds DESC, peerId ASC, messageId DESC",
        MediaSort.FOLDER_DESC to "(peerName IS NULL) ASC, peerName COLLATE NOCASE DESC, dateEpochSeconds DESC, peerId ASC, messageId DESC",
    )

    fun build(accountId: Long, search: String, filter: MediaFilter, sort: MediaSort): SupportSQLiteQuery {
        val sql = StringBuilder("SELECT * FROM telegram_media WHERE accountId = ? AND deleted = 0")
        val args = mutableListOf<Any>(accountId)
        val normalizedSearch = SearchNormalizer.normalize(search)
        if (normalizedSearch.isNotEmpty()) {
            sql.append(" AND normalizedSearchText LIKE ? ESCAPE '\\'")
            args += "%${SearchNormalizer.escapeLike(normalizedSearch)}%"
        }
        when (filter.scope) {
            MediaScope.ALL -> Unit
            MediaScope.IMAGES -> sql.append(" AND mediaType IN ('IMAGE','ANIMATED_IMAGE')")
            MediaScope.VIDEOS -> sql.append(" AND mediaType = 'VIDEO'")
        }
        filter.peerId?.let { sql.append(" AND peerId = ?"); args += it }
        filter.dateFromEpochSeconds?.let { sql.append(" AND dateEpochSeconds >= ?"); args += it }
        filter.dateToEpochSeconds?.let { sql.append(" AND dateEpochSeconds <= ?"); args += it }
        filter.minimumSizeBytes?.let { sql.append(" AND sizeBytes IS NOT NULL AND sizeBytes >= ?"); args += it }
        filter.maximumSizeBytes?.let { sql.append(" AND sizeBytes IS NOT NULL AND sizeBytes <= ?"); args += it }
        filter.minimumDurationSeconds?.let { sql.append(" AND durationSeconds IS NOT NULL AND durationSeconds >= ?"); args += it }
        filter.maximumDurationSeconds?.let { sql.append(" AND durationSeconds IS NOT NULL AND durationSeconds <= ?"); args += it }
        filter.extension?.trim()?.lowercase(Locale.ROOT)?.takeIf(String::isNotEmpty)?.let {
            sql.append(" AND extension = ?")
            args += it.removePrefix(".")
        }
        filter.mimeType?.trim()?.lowercase(Locale.ROOT)?.takeIf(String::isNotEmpty)?.let {
            sql.append(" AND mimeType = ?")
            args += it.substringBefore(';')
        }
        when (filter.thumbnail) {
            ThumbnailFilter.ANY -> Unit
            ThumbnailFilter.HAS_THUMBNAIL -> sql.append(" AND thumbnailAvailable = 1")
            ThumbnailFilter.NO_THUMBNAIL -> sql.append(" AND thumbnailAvailable = 0")
        }
        when (filter.resolution) {
            ResolutionFilter.ANY -> Unit
            ResolutionFilter.SMALL -> sql.append(" AND width IS NOT NULL AND height IS NOT NULL AND MAX(width, height) < 1280")
            ResolutionFilter.HD -> sql.append(" AND width IS NOT NULL AND height IS NOT NULL AND MAX(width, height) >= 1280 AND MAX(width, height) < 1920")
            ResolutionFilter.FULL_HD -> sql.append(" AND width IS NOT NULL AND height IS NOT NULL AND MAX(width, height) >= 1920 AND MAX(width, height) < 3840")
            ResolutionFilter.FOUR_K -> sql.append(" AND width IS NOT NULL AND height IS NOT NULL AND MAX(width, height) >= 3840")
        }
        sql.append(" ORDER BY ").append(sortClauses.getValue(sort))
        return SimpleSQLiteQuery(sql.toString(), args.toTypedArray())
    }
}

