package com.cameronamer.telegramdrive.nativeplayer

import java.lang.ref.WeakReference
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

data class NativePlayerSession(
    val id: String,
    val args: OpenNativePlayerArgs,
    var stateListener: ((NativePlaybackSnapshot) -> Unit)? = null,
)

object NativePlayerSessionStore {
    private val sessions = ConcurrentHashMap<String, NativePlayerSession>()

    fun create(args: OpenNativePlayerArgs): NativePlayerSession {
        val session = NativePlayerSession(UUID.randomUUID().toString(), args)
        sessions[session.id] = session
        return session
    }

    fun get(id: String?): NativePlayerSession? = id?.let(sessions::get)

    fun remove(id: String?) {
        id?.let(sessions::remove)?.stateListener = null
    }
}
internal class WeakInstanceRegistry<T : Any> {
    private var reference: WeakReference<T>? = null

    @Synchronized
    fun register(instance: T) {
        reference = WeakReference(instance)
    }

    @Synchronized
    fun clear(instance: T? = null) {
        if (instance == null || reference?.get() === instance) reference = null
    }

    @Synchronized
    fun get(): T? {
        val value = reference?.get()
        if (value == null) reference = null
        return value
    }
}

object NativePlayerActivityRegistry {
    private val registry = WeakInstanceRegistry<NativePlayerActivity>()

    fun register(activity: NativePlayerActivity) = registry.register(activity)
    fun clear(activity: NativePlayerActivity? = null) = registry.clear(activity)
    fun close() = registry.get()?.finishFromExternal()
    fun snapshot(): NativePlaybackSnapshot = registry.get()?.playbackSnapshot()
        ?: NativePlaybackSnapshot()
}
