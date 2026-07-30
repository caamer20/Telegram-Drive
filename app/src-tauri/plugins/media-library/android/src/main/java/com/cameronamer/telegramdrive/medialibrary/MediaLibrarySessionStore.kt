package com.cameronamer.telegramdrive.medialibrary

import java.lang.ref.WeakReference
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

data class MediaLibrarySession(
    val id: String,
    val baseUrl: String,
    val authorizationToken: String,
    @Volatile var accountId: Long? = null,
)

object MediaLibrarySessionStore {
    private val sessions = ConcurrentHashMap<String, MediaLibrarySession>()

    fun create(args: OpenMediaLibraryArgs): MediaLibrarySession {
        args.validate()
        val session = MediaLibrarySession(
            UUID.randomUUID().toString(),
            args.baseUrl,
            args.authorizationToken,
        )
        sessions[session.id] = session
        return session
    }

    fun get(id: String?): MediaLibrarySession? = id?.let(sessions::get)

    fun remove(id: String?) {
        id?.let(sessions::remove)
    }

    fun clear() = sessions.clear()
}

object MediaLibraryRuntimeState {
    @Volatile var accountId: Long? = null
    @Volatile var online: Boolean = false
    @Volatile var syncRunning: Boolean = false
    @Volatile var opening: Boolean = false

    fun snapshot(isOpen: Boolean): MediaLibraryStateData {
        val status = when {
            opening -> "opening"
            !isOpen -> "closed"
            !online -> "offline"
            else -> "open"
        }
        return MediaLibraryStateData(status, isOpen, accountId, online, syncRunning)
    }

    fun resetCredentials() {
        online = false
        syncRunning = false
        opening = false
    }
}

object MediaLibraryActivityRegistry {
    private var activity: WeakReference<MediaLibraryActivity>? = null
    private val closeRequested = AtomicBoolean(false)

    @Synchronized
    fun register(value: MediaLibraryActivity) {
        activity = WeakReference(value)
        if (closeRequested.getAndSet(false)) value.finishFromExternal()
    }

    @Synchronized
    fun clear(value: MediaLibraryActivity? = null) {
        if (value == null || activity?.get() === value) activity = null
    }

    @Synchronized
    fun isOpen(): Boolean = activity?.get() != null

    fun close() {
        val current = synchronized(this) { activity?.get() }
        if (current == null) closeRequested.set(true) else current.runOnUiThread(current::finishFromExternal)
    }

    fun clearPendingClose() = closeRequested.set(false)
}

