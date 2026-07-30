package com.cameronamer.telegramdrive.medialibrary.network

import com.cameronamer.telegramdrive.medialibrary.data.MediaType
import com.cameronamer.telegramdrive.medialibrary.data.SearchNormalizer
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaEntity
import com.cameronamer.telegramdrive.medialibrary.data.ThumbnailStatus
import kotlinx.coroutines.suspendCancellableCoroutine
import okhttp3.Call
import okhttp3.Callback
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

data class AccountDto(val accountId: Long, val displayName: String?)
data class PeerDto(val peerId: Long, val folderId: Long?, val name: String, val kind: String)
data class MediaPageDto(
    val items: List<MediaRecordDto>,
    val nextOffsetMessageId: Int?,
    val hasMore: Boolean,
    val messagesScanned: Int,
    val mediaFound: Int,
    val newestScannedMessageId: Int?,
    val oldestScannedMessageId: Int?,
    val reachedNewerThanBoundary: Boolean,
)

data class MediaRecordDto(
    val accountId: Long,
    val peerId: Long,
    val folderId: Long?,
    val messageId: Int,
    val peerName: String?,
    val senderId: Long?,
    val dateEpochSeconds: Long,
    val displayName: String,
    val originalFilename: String?,
    val caption: String?,
    val mediaType: String,
    val mimeType: String?,
    val extension: String?,
    val sizeBytes: Long?,
    val durationSeconds: Int?,
    val width: Int?,
    val height: Int?,
    val thumbnailAvailable: Boolean,
    val thumbnailVariant: String?,
) {
    fun toEntity(nowEpochSeconds: Long, existing: TelegramMediaEntity? = null): TelegramMediaEntity {
        val mappedType = when (mediaType.lowercase()) {
            "video" -> MediaType.VIDEO
            "animated-image" -> MediaType.ANIMATED_IMAGE
            else -> MediaType.IMAGE
        }
        val preserveThumbnail = existing?.thumbnailPath != null &&
            existing.thumbnailVariant == thumbnailVariant &&
            thumbnailAvailable
        val normalizedName = SearchNormalizer.normalize(displayName)
        val normalizedSearch = SearchNormalizer.normalize(
            listOfNotNull(displayName, originalFilename, caption, peerName).joinToString(" "),
        )
        return TelegramMediaEntity(
            accountId = accountId,
            peerId = peerId,
            messageId = messageId,
            folderId = folderId,
            peerName = peerName,
            senderId = senderId,
            displayName = displayName,
            originalFilename = originalFilename,
            normalizedName = normalizedName,
            normalizedSearchText = normalizedSearch,
            caption = caption,
            mediaType = mappedType,
            mimeType = mimeType?.lowercase(),
            extension = extension?.lowercase()?.removePrefix("."),
            sizeBytes = sizeBytes,
            dateEpochSeconds = dateEpochSeconds,
            durationSeconds = durationSeconds,
            width = width,
            height = height,
            thumbnailAvailable = thumbnailAvailable,
            thumbnailVariant = thumbnailVariant,
            thumbnailPath = if (preserveThumbnail) existing?.thumbnailPath else null,
            thumbnailStatus = when {
                !thumbnailAvailable -> ThumbnailStatus.NO_THUMBNAIL
                preserveThumbnail -> ThumbnailStatus.READY
                else -> ThumbnailStatus.NOT_REQUESTED
            },
            lastSyncedAtEpochSeconds = nowEpochSeconds,
            deleted = false,
        )
    }
}

sealed class MediaApiException(message: String) : IOException(message) {
    class SessionExpired : MediaApiException("The private media session expired")
    class RuntimeUnavailable : MediaApiException("The Telegram runtime is unavailable")
    class InvalidResponse : MediaApiException("The local media service returned an invalid response")
    class RequestFailed : MediaApiException("The local media request failed")
}

class MediaLibraryApi(
    private val baseUrl: String,
    private val authorizationToken: String,
    private val client: OkHttpClient = defaultClient(),
) {
    init {
        require(Regex("^http://127\\.0\\.0\\.1:[1-9][0-9]{0,4}$").matches(baseUrl))
        require(authorizationToken.isNotEmpty() && authorizationToken.length <= 512)
    }

    suspend fun account(): AccountDto {
        val json = executeJson(request("/native-media-library/v1/account"))
        return AccountDto(json.getLong("accountId"), json.optNullableString("displayName"))
    }

    suspend fun peers(): List<PeerDto> {
        val items = executeJson(request("/native-media-library/v1/peers")).getJSONArray("items")
        return items.objects().map {
            PeerDto(
                peerId = it.getLong("peerId"),
                folderId = it.optNullableLong("folderId"),
                name = it.getString("name"),
                kind = it.getString("kind"),
            )
        }
    }

    suspend fun mediaPage(
        folderId: Long?,
        offsetMessageId: Int,
        limit: Int = 200,
        newerThanMessageId: Int? = null,
    ): MediaPageDto {
        require(folderId == null || folderId > 0)
        require(offsetMessageId >= 0)
        require(limit in 1..200)
        val body = JSONObject()
            .put("folderId", folderId ?: JSONObject.NULL)
            .put("offsetMessageId", offsetMessageId)
            .put("limit", limit)
            .put("newerThanMessageId", newerThanMessageId ?: JSONObject.NULL)
            .toString()
            .toRequestBody(JSON)
        val json = executeJson(request("/native-media-library/v1/media-page").post(body))
        val items = json.getJSONArray("items").objects().map(::parseMediaRecord)
        return MediaPageDto(
            items = items,
            nextOffsetMessageId = json.optNullableInt("nextOffsetMessageId"),
            hasMore = json.getBoolean("hasMore"),
            messagesScanned = json.getInt("messagesScanned"),
            mediaFound = json.getInt("mediaFound"),
            newestScannedMessageId = json.optNullableInt("newestScannedMessageId"),
            oldestScannedMessageId = json.optNullableInt("oldestScannedMessageId"),
            reachedNewerThanBoundary = json.optBoolean("reachedNewerThanBoundary", false),
        )
    }

    suspend fun thumbnail(folderId: Long?, messageId: Int, targetPx: Int): Response {
        require(folderId == null || folderId > 0)
        require(messageId > 0)
        val folder = folderId?.toString() ?: "home"
        val response = client.newCall(
            request("/native-media-library/v1/thumbnail/$folder/$messageId?targetPx=${targetPx.coerceIn(64, 1024)}")
                .build(),
        ).await()
        if (response.code in listOf(401, 403, 503)) {
            val code = response.code
            response.close()
            throw if (code == 503) MediaApiException.RuntimeUnavailable() else MediaApiException.SessionExpired()
        }
        return response
    }

    private fun request(path: String): Request.Builder = Request.Builder()
        .url(baseUrl + path)
        .header("Authorization", "Bearer $authorizationToken")
        .header("Accept", "application/json")

    private suspend fun executeJson(builder: Request.Builder): JSONObject {
        val response = execute(builder.build())
        response.use {
            val text = it.body?.string() ?: throw MediaApiException.InvalidResponse()
            return try {
                JSONObject(text)
            } catch (_: Exception) {
                throw MediaApiException.InvalidResponse()
            }
        }
    }

    private suspend fun execute(request: Request): Response {
        val response = client.newCall(request).await()
        if (response.isSuccessful) return response
        val code = response.code
        response.close()
        throw when (code) {
            401, 403 -> MediaApiException.SessionExpired()
            503 -> MediaApiException.RuntimeUnavailable()
            else -> MediaApiException.RequestFailed()
        }
    }

    private fun parseMediaRecord(value: JSONObject): MediaRecordDto = MediaRecordDto(
        accountId = value.getLong("accountId"),
        peerId = value.getLong("peerId"),
        folderId = value.optNullableLong("folderId"),
        messageId = value.getInt("messageId"),
        peerName = value.optNullableString("peerName"),
        senderId = value.optNullableLong("senderId"),
        dateEpochSeconds = value.getLong("dateEpochSeconds"),
        displayName = value.getString("displayName"),
        originalFilename = value.optNullableString("originalFilename"),
        caption = value.optNullableString("caption"),
        mediaType = value.getString("mediaType"),
        mimeType = value.optNullableString("mimeType"),
        extension = value.optNullableString("extension"),
        sizeBytes = value.optNullableLong("sizeBytes"),
        durationSeconds = value.optNullableInt("durationSeconds"),
        width = value.optNullableInt("width"),
        height = value.optNullableInt("height"),
        thumbnailAvailable = value.getBoolean("thumbnailAvailable"),
        thumbnailVariant = value.optNullableString("thumbnailVariant"),
    )

    internal fun serializableStateForTest(): Map<String, Any> = emptyMap()

    companion object {
        private val JSON = "application/json; charset=utf-8".toMediaType()
        fun defaultClient(): OkHttpClient = OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .writeTimeout(10, TimeUnit.SECONDS)
            .callTimeout(45, TimeUnit.SECONDS)
            .retryOnConnectionFailure(false)
            .build()
    }
}

private suspend fun Call.await(): Response = suspendCancellableCoroutine { continuation ->
    continuation.invokeOnCancellation { cancel() }
    enqueue(object : Callback {
        override fun onFailure(call: Call, error: IOException) {
            if (continuation.isActive) continuation.resumeWithException(MediaApiException.RequestFailed())
        }

        override fun onResponse(call: Call, response: Response) {
            if (continuation.isActive) continuation.resume(response) else response.close()
        }
    })
}

private fun JSONArray.objects(): List<JSONObject> = (0 until length()).map(::getJSONObject)
private fun JSONObject.optNullableString(key: String): String? =
    if (isNull(key) || !has(key)) null else getString(key)
private fun JSONObject.optNullableLong(key: String): Long? =
    if (isNull(key) || !has(key)) null else getLong(key)
private fun JSONObject.optNullableInt(key: String): Int? =
    if (isNull(key) || !has(key)) null else getInt(key)
