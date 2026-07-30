package com.nmtuong.telegramdrive.data

import com.nmtuong.telegramdrive.data.fake.FakeTelegramCatalog
import com.nmtuong.telegramdrive.domain.*
import com.nmtuong.telegramdrive.telegram.TdLibGateway
import java.io.File
import java.util.Base64
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

class RealTelegramRepository(private val gateway: TdLibGateway) : TelegramRepository {
  override val diagnostics = gateway.state
  override val authorization = gateway.authorization
  override val library = gateway.library
  override fun start() = gateway.start()
  override fun submit(action: AuthorizationAction) = gateway.submit(action)
  override fun loadSavedMessages(limit: Int) = gateway.loadSavedMessages(limit)
  override fun download(fileId: Int) = gateway.download(fileId)
  override fun cancelDownload(fileId: Int) = gateway.cancelDownload(fileId)
  override fun preview(itemId: Long) = gateway.preview(itemId)
  override fun close() = gateway.close()
}

class FakeTelegramRepository(
  val catalog: FakeTelegramCatalog,
  private val filesDirectory: File = File(System.getProperty("java.io.tmpdir"), "telegram-drive-fake"),
  private val videoBytes: () -> ByteArray = { ByteArray(0) },
  dispatcher: CoroutineDispatcher = Dispatchers.Default,
  private val downloadStepDelayMillis: Long = 150,
) : TelegramRepository {
  private val lock = Any()
  private val scope = CoroutineScope(SupervisorJob() + dispatcher)
  private val downloadJobs = mutableMapOf<Int, Job>()
  private val downloadGenerations = mutableMapOf<Int, Long>()
  private val mutableDiagnostics = MutableStateFlow(
    DiagnosticsState(
      dataSource = DataSourceMode.FAKE,
      lifecycle = GatewayLifecycle.RUNNING,
      authorizationState = AuthorizationState.WaitingForPhoneNumber,
    ),
  )
  override val diagnostics: StateFlow<DiagnosticsState> = mutableDiagnostics
  private val mutableAuthorization = MutableStateFlow(AuthorizationSession(AuthorizationState.WaitingForPhoneNumber))
  override val authorization: StateFlow<AuthorizationSession> = mutableAuthorization
  private val mutableLibrary = MutableStateFlow<LibraryState>(LibraryState.Idle)
  override val library: StateFlow<LibraryState> = mutableLibrary

  override fun start() = Unit

  override fun submit(action: AuthorizationAction): ActionResult {
    val next = when (action) {
      is AuthorizationAction.SubmitPhone -> if (authorization.value.state == AuthorizationState.WaitingForPhoneNumber) AuthorizationState.WaitingForCode else null
      is AuthorizationAction.SubmitCode -> if (authorization.value.state == AuthorizationState.WaitingForCode) AuthorizationState.WaitingForPassword("fake 2FA") else null
      is AuthorizationAction.SubmitPassword -> if (authorization.value.state is AuthorizationState.WaitingForPassword) AuthorizationState.Ready else null
      is AuthorizationAction.SubmitEmailAddress -> if (authorization.value.state == AuthorizationState.WaitingForEmailAddress) AuthorizationState.WaitingForEmailCode else null
      is AuthorizationAction.SubmitEmailCode -> if (authorization.value.state == AuthorizationState.WaitingForEmailCode) AuthorizationState.Ready else null
      AuthorizationAction.Logout -> if (authorization.value.state == AuthorizationState.Ready) AuthorizationState.Closed else null
      AuthorizationAction.Reset -> AuthorizationState.Closed
    } ?: return ActionResult.INVALID_STATE
    mutableAuthorization.value = AuthorizationSession(next)
    mutableDiagnostics.value = mutableDiagnostics.value.copy(authorizationState = next)
    if (next != AuthorizationState.Ready) {
      cancelDownloadsAndClearFiles()
      mutableLibrary.value = LibraryState.Idle
    }
    return ActionResult.ACCEPTED
  }

  override fun loadSavedMessages(limit: Int): ActionResult {
    if (authorization.value.state != AuthorizationState.Ready) return ActionResult.INVALID_STATE
    val supportedKinds = setOf(MediaKind.IMAGE, MediaKind.VIDEO, MediaKind.ANIMATION, MediaKind.DOCUMENT)
    val items = catalog.media.filter {
      it.sourceId == catalog.sources.first { source -> source.savedMessages }.id && it.kind in supportedKinds
    }
      .distinctBy { it.fileId }
      .take(limit.coerceIn(1, 50))
      .map { it.copy(downloadState = DownloadState.NotDownloaded, localPath = null) }
    mutableLibrary.value = if (items.isEmpty()) LibraryState.Empty else LibraryState.Content(items)
    return ActionResult.ACCEPTED
  }

  override fun download(fileId: Int): ActionResult {
    val content = library.value as? LibraryState.Content ?: return ActionResult.INVALID_STATE
    val item = content.items.firstOrNull { it.fileId == fileId } ?: return ActionResult.INVALID_STATE
    if (item.downloadState is DownloadState.Downloading) return ActionResult.DUPLICATE
    if (item.downloadState == DownloadState.Complete && item.localPath?.let(::File)?.isFile == true) return ActionResult.DUPLICATE
    val generation = synchronized(lock) {
      if (downloadJobs[fileId]?.isActive == true) return ActionResult.DUPLICATE
      (downloadGenerations[fileId] ?: 0L) + 1L
    }
    synchronized(lock) { downloadGenerations[fileId] = generation }
    updateItem(fileId) { it.copy(downloadState = DownloadState.Downloading(0), localPath = null) }
    val job = scope.launch {
      delay(downloadStepDelayMillis)
      if (!isCurrentDownload(fileId, generation)) return@launch
      updateItem(fileId) { it.copy(downloadState = DownloadState.Downloading(50)) }
      delay(downloadStepDelayMillis)
      if (!isCurrentDownload(fileId, generation)) return@launch
      if (item.kind == MediaKind.ANIMATION) {
        updateItem(fileId) { it.copy(downloadState = DownloadState.Failed("Simulated download failure"), localPath = null) }
      } else {
        filesDirectory.mkdirs()
        val target = File(filesDirectory, item.name)
        when (item.kind) {
          MediaKind.IMAGE -> target.writeBytes(FAKE_PNG)
          MediaKind.VIDEO -> target.writeBytes(videoBytes())
          else -> target.writeBytes(ByteArray(0))
        }
        if (isCurrentDownload(fileId, generation)) {
          updateItem(fileId) { it.copy(downloadState = DownloadState.Complete, localPath = target.absolutePath) }
        }
      }
      synchronized(lock) { downloadJobs.remove(fileId) }
    }
    synchronized(lock) { downloadJobs[fileId] = job }
    return ActionResult.ACCEPTED
  }

  override fun cancelDownload(fileId: Int): ActionResult {
    val item = (library.value as? LibraryState.Content)?.items?.firstOrNull { it.fileId == fileId }
      ?: return ActionResult.INVALID_STATE
    if (item.downloadState !is DownloadState.Downloading) return ActionResult.INVALID_STATE
    synchronized(lock) {
      downloadGenerations[fileId] = (downloadGenerations[fileId] ?: 0L) + 1L
      downloadJobs.remove(fileId)?.cancel()
    }
    updateItem(fileId) { it.copy(downloadState = DownloadState.Canceled, localPath = null) }
    return ActionResult.ACCEPTED
  }

  override fun preview(itemId: Long): PreviewTarget? {
    val item = (library.value as? LibraryState.Content)?.items?.firstOrNull { it.id == itemId } ?: return null
    val path = item.localPath?.takeIf { File(it).isFile } ?: return null
    return when (item.kind) {
      MediaKind.IMAGE -> PreviewTarget.Image(item.id, path)
      MediaKind.VIDEO -> PreviewTarget.Video(item.id, path)
      else -> null
    }
  }

  override fun close() {
    synchronized(lock) { downloadJobs.values.forEach(Job::cancel); downloadJobs.clear() }
    scope.cancel()
  }

  private fun isCurrentDownload(fileId: Int, generation: Long): Boolean =
    synchronized(lock) { downloadGenerations[fileId] == generation }

  private fun updateItem(fileId: Int, transform: (MediaItem) -> MediaItem) {
    synchronized(lock) {
      val content = mutableLibrary.value as? LibraryState.Content ?: return
      mutableLibrary.value = LibraryState.Content(content.items.map { if (it.fileId == fileId) transform(it) else it })
    }
  }

  private fun cancelDownloadsAndClearFiles() {
    synchronized(lock) {
      downloadJobs.values.forEach(Job::cancel)
      downloadJobs.clear()
      downloadGenerations.replaceAll { _, generation -> generation + 1L }
    }
    filesDirectory.deleteRecursively()
  }

  private companion object {
    val FAKE_PNG: ByteArray = Base64.getDecoder().decode(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
    )
  }
}
