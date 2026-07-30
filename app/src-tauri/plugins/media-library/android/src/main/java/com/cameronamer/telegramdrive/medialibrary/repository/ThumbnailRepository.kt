package com.cameronamer.telegramdrive.medialibrary.repository

import android.content.Context
import android.system.Os
import com.cameronamer.telegramdrive.medialibrary.data.MediaDao
import com.cameronamer.telegramdrive.medialibrary.data.TelegramMediaEntity
import com.cameronamer.telegramdrive.medialibrary.data.ThumbnailStatus
import com.cameronamer.telegramdrive.medialibrary.network.MediaLibraryApi
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import java.io.File
import java.io.FileOutputStream
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger

class ThumbnailCache(
    context: Context,
    private val dao: MediaDao,
    private val maximumBytes: Long = DEFAULT_MAXIMUM_BYTES,
) {
    private val root = File(context.filesDir, "media-thumbnails")
    private val protected = ConcurrentHashMap.newKeySet<String>()
    private val retained = ConcurrentHashMap<String, AtomicInteger>()
    private val pendingAccountClears = ConcurrentHashMap.newKeySet<String>()

    fun cacheFile(item: TelegramMediaEntity): File {
        val variant = sanitize(item.thumbnailVariant ?: "default")
        val account = item.accountId.toString()
        val directory = File(root, account)
        check(directory.canonicalPath.startsWith(root.canonicalPath + File.separator))
        return File(directory, "${item.peerId}_${item.messageId}_$variant.jpg")
    }

    fun validate(file: File?): Boolean {
        if (file == null || !file.isFile || file.length() <= 0) return false
        val rootPath = root.canonicalPath + File.separator
        return file.canonicalPath.startsWith(rootPath)
    }

    fun touch(file: File) {
        if (validate(file)) file.setLastModified(System.currentTimeMillis())
    }

    fun retain(file: File) {
        if (validate(file)) retained.computeIfAbsent(file.absolutePath) { AtomicInteger() }.incrementAndGet()
    }

    fun release(file: File) {
        var released = false
        retained.computeIfPresent(file.absolutePath) { _, count ->
            if (count.decrementAndGet() <= 0) {
                released = true
                null
            } else count
        }
        if (released) deleteIfAccountClearPending(file)
    }

    suspend fun writeAtomic(item: TelegramMediaEntity, bytes: ByteArray): File = withContext(Dispatchers.IO) {
        require(bytes.isNotEmpty() && bytes.size <= MAX_SINGLE_FILE_BYTES)
        val target = cacheFile(item)
        target.parentFile?.mkdirs()
        protected += target.absolutePath
        val temporary = File(target.parentFile, ".${target.name}.${UUID.randomUUID()}.part")
        try {
            FileOutputStream(temporary).use { output ->
                output.write(bytes)
                output.fd.sync()
            }
            check(temporary.length() == bytes.size.toLong())
            // POSIX rename replaces an existing file atomically on the same
            // app-private filesystem and is available below java.nio API 26.
            Os.rename(temporary.absolutePath, target.absolutePath)
            // Some instrumented/test filesystems expose rename as a successful
            // no-op. Verify the source was consumed and retain a safe fallback
            // for those environments instead of recording a nonexistent file.
            if (temporary.exists()) {
                if (target.exists()) check(target.delete())
                check(temporary.renameTo(target))
            }
            check(target.isFile && target.length() == bytes.size.toLong())
            target.setLastModified(System.currentTimeMillis())
            evictIfNeeded(target)
            target
        } finally {
            temporary.delete()
            protected -= target.absolutePath
            deleteIfAccountClearPending(target)
        }
    }

    suspend fun evictIfNeeded(recent: File? = null) = withContext(Dispatchers.IO) {
        val files = root.walkTopDown().filter(File::isFile).filterNot { it.name.endsWith(".part") }.toList()
        var total = files.sumOf(File::length)
        if (total <= maximumBytes) return@withContext
        for (file in files.sortedBy(File::lastModified)) {
            if (total <= maximumBytes) break
            if (file == recent || file.absolutePath in protected || retained.containsKey(file.absolutePath)) continue
            val size = file.length()
            val path = file.absolutePath
            if (file.delete()) {
                total -= size
                dao.markThumbnailEvicted(path)
            }
        }
    }

    suspend fun clearAccount(accountId: Long) = withContext(Dispatchers.IO) {
        val account = accountId.toString()
        pendingAccountClears += account
        val directory = File(root, accountId.toString())
        if (!directory.exists()) {
            pendingAccountClears -= account
            return@withContext
        }
        check(directory.canonicalPath.startsWith(root.canonicalPath + File.separator))
        directory.walkBottomUp().forEach { file ->
            if (file.absolutePath !in protected && !retained.containsKey(file.absolutePath)) file.delete()
        }
        if (!directory.exists() || directory.listFiles().isNullOrEmpty()) {
            directory.delete()
            pendingAccountClears -= account
        }
    }

    private fun deleteIfAccountClearPending(file: File) {
        val account = file.parentFile?.name ?: return
        if (account !in pendingAccountClears || file.absolutePath in protected || retained.containsKey(file.absolutePath)) return
        file.delete()
        val directory = file.parentFile ?: return
        if (directory.listFiles().isNullOrEmpty()) {
            directory.delete()
            pendingAccountClears -= account
        }
    }

    private fun sanitize(value: String): String = value
        .lowercase()
        .map { if (it.isLetterOrDigit() || it == '-' || it == '_') it else '_' }
        .joinToString("")
        .take(32)
        .ifEmpty { "default" }

    companion object {
        const val DEFAULT_MAXIMUM_BYTES = 256L * 1024 * 1024
        private const val MAX_SINGLE_FILE_BYTES = 12 * 1024 * 1024
    }
}

class ThumbnailRepository(
    private val api: MediaLibraryApi?,
    private val dao: MediaDao,
    private val cache: ThumbnailCache,
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO),
) {
    private val semaphore = Semaphore(4)
    private val requests = ConcurrentHashMap<String, Deferred<File?>>()

    fun retain(file: File) = cache.retain(file)
    fun release(file: File) = cache.release(file)

    fun cancelAll() {
        requests.values.forEach { it.cancel() }
        requests.clear()
        scope.cancel()
    }

    suspend fun ensureThumbnail(
        item: TelegramMediaEntity,
        explicitRetry: Boolean = false,
        targetPx: Int = 320,
    ): File? {
        item.thumbnailPath?.let(::File)?.takeIf(cache::validate)?.let {
            cache.touch(it)
            return it
        }
        if (!item.thumbnailAvailable) {
            dao.updateThumbnail(item.accountId, item.peerId, item.messageId, ThumbnailStatus.NO_THUMBNAIL, null)
            return null
        }
        if (api == null || (item.thumbnailStatus == ThumbnailStatus.FAILED && !explicitRetry)) return null
        val boundedTarget = targetPx.coerceIn(96, 1024)
        val key = item.stableKey
        val deferred = requests.computeIfAbsent(key) {
            scope.async { download(item, boundedTarget) }
        }
        return try {
            deferred.await()
        } finally {
            requests.remove(key, deferred)
        }
    }

    private suspend fun download(item: TelegramMediaEntity, targetPx: Int): File? = semaphore.withPermit {
        dao.updateThumbnail(item.accountId, item.peerId, item.messageId, ThumbnailStatus.LOADING, null)
        repeat(3) { attempt ->
            try {
                val response = api!!.thumbnail(item.folderId, item.messageId, targetPx)
                response.use {
                    if (it.code == 404) {
                        dao.updateThumbnail(item.accountId, item.peerId, item.messageId, ThumbnailStatus.NO_THUMBNAIL, null)
                        return null
                    }
                    if (!it.isSuccessful) throw java.io.IOException("thumbnail request failed")
                    val bytes = it.body?.bytes() ?: throw java.io.IOException("empty thumbnail")
                    if (!isSupportedThumbnailPayload(it.header("Content-Type"), bytes)) {
                        throw java.io.IOException("invalid thumbnail response")
                    }
                    val file = cache.writeAtomic(item, bytes)
                    dao.updateThumbnail(item.accountId, item.peerId, item.messageId, ThumbnailStatus.READY, file.absolutePath)
                    return file
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                dao.updateThumbnail(item.accountId, item.peerId, item.messageId, ThumbnailStatus.NOT_REQUESTED, null)
                throw cancelled
            } catch (_: Exception) {
                if (attempt < 2) delay(RETRY_DELAYS_MS[attempt])
            }
        }
        dao.updateThumbnail(item.accountId, item.peerId, item.messageId, ThumbnailStatus.FAILED, null)
        null
    }

    internal fun inFlightCountForTest(): Int = requests.size

    companion object {
        val RETRY_DELAYS_MS = longArrayOf(250, 750)

        internal fun isSupportedThumbnailPayload(contentType: String?, bytes: ByteArray): Boolean {
            if (contentType?.substringBefore(';')?.trim()?.lowercase()?.startsWith("image/") != true) return false
            return bytes.size >= 3 && (
                bytes[0] == 0xFF.toByte() && bytes[1] == 0xD8.toByte() && bytes[2] == 0xFF.toByte()
                    || bytes.size >= 8 && bytes.copyOfRange(0, 8).contentEquals(byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A))
                    || bytes.size >= 6 && (String(bytes, 0, 6, Charsets.US_ASCII) == "GIF87a" || String(bytes, 0, 6, Charsets.US_ASCII) == "GIF89a")
                    || bytes.size >= 12 && String(bytes, 0, 4, Charsets.US_ASCII) == "RIFF" && String(bytes, 8, 4, Charsets.US_ASCII) == "WEBP"
                )
        }
    }
}
