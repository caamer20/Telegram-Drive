package com.cameronamer.telegramdrive.medialibrary

import app.tauri.annotation.InvokeArg
import app.tauri.plugin.JSObject

@InvokeArg
class OpenMediaLibraryArgs {
    lateinit var baseUrl: String
    lateinit var authorizationToken: String

    fun validate() {
        require(LOOPBACK_BASE.matches(baseUrl)) { "only trusted IPv4 loopback sessions are allowed" }
        require(authorizationToken.isNotEmpty() && authorizationToken.length <= 512) {
            "media library credentials are invalid"
        }
    }

    companion object {
        private val LOOPBACK_BASE = Regex("^http://127\\.0\\.0\\.1:[1-9][0-9]{0,4}$")
    }
}

@InvokeArg
class ClearMediaLibraryArgs {
    var accountId: Long? = null
}

data class MediaLibraryResultData(
    val exitReason: String = "back",
    val accountId: Long? = null,
    val error: String? = null,
) {
    fun toJsObject(): JSObject = JSObject().apply {
        put("exitReason", exitReason)
        accountId?.let { put("accountId", it) } ?: put("accountId", null)
        error?.let { put("error", it) }
    }
}

data class MediaLibraryStateData(
    val status: String = "closed",
    val isOpen: Boolean = false,
    val accountId: Long? = null,
    val online: Boolean = false,
    val syncRunning: Boolean = false,
) {
    fun toJsObject(): JSObject = JSObject().apply {
        put("status", status)
        put("isOpen", isOpen)
        accountId?.let { put("accountId", it) } ?: put("accountId", null)
        put("online", online)
        put("syncRunning", syncRunning)
    }
}

