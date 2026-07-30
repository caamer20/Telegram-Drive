package com.nmtuong.telegramdrive.domain

enum class DataSourceMode(val id: String) { REAL("real"), FAKE("fake") }

sealed interface AuthorizationState {
  data object Unknown : AuthorizationState
  data object WaitingForTdlibParameters : AuthorizationState
  data object WaitingForPhoneNumber : AuthorizationState
  data object Ready : AuthorizationState
  data object Closed : AuthorizationState
  data class Other(val name: String) : AuthorizationState
}

data class DiagnosticsState(
  val dataSource: DataSourceMode,
  val nativeLibraryLoaded: Boolean = false,
  val clientCreated: Boolean = false,
  val authorizationState: AuthorizationState = AuthorizationState.Unknown,
  val safeError: String? = null,
  val clientInstanceCount: Int = 0,
)

data class Account(val id: Long, val displayName: String)
data class FileSource(val id: Long, val title: String, val savedMessages: Boolean)
enum class MediaKind { IMAGE, VIDEO, AUDIO, PDF, DOCUMENT }
sealed interface DownloadState {
  data class Downloading(val percent: Int) : DownloadState
  data object Complete : DownloadState
  data class Failed(val reason: String) : DownloadState
}
data class MediaItem(
  val id: Long,
  val sourceId: Long,
  val name: String,
  val kind: MediaKind,
  val downloadState: DownloadState,
)
