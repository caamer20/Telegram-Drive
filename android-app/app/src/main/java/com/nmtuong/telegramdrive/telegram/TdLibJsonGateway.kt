package com.nmtuong.telegramdrive.telegram

import android.content.Context
import com.nmtuong.telegramdrive.domain.*
import com.nmtuong.telegramdrive.security.SensitiveDataRedactor
import com.nmtuong.telegramdrive.security.TelegramApiConfiguration
import java.io.File
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.json.*

class TdLibJsonGateway internal constructor(
  context: Context? = null,
  private val configuration: TelegramApiConfiguration = TelegramApiConfiguration(0, ""),
  private val native: TdLibNative = JsonTdLibNative,
  private val libraryLoader: NativeLibraryLoader = NativeLibraryLoader { System.loadLibrary("tdjsonjava") },
  dispatcher: CoroutineDispatcher = Dispatchers.IO,
) : TdLibGateway {
  private val lock = Any()
  private val scope = CoroutineScope(SupervisorJob() + dispatcher)
  private val json = Json { ignoreUnknownKeys = true }
  private val databaseDirectory = context?.filesDir?.resolve("tdlib/database") ?: File("tdlib-test/database")
  private val filesDirectory = context?.filesDir?.resolve("tdlib/files") ?: File("tdlib-test/files")
  private var lifecycle = GatewayLifecycle.NEW
  private var worker: Job? = null
  private var clientId: Int? = null
  private var countedClient = false
  private var pendingAuthAction = false
  private var pendingAuthRequest: String? = null
  private var pendingParametersRequest: String? = null
  private var pendingLibraryRequest: String? = null
  private var pendingHistoryLimit = 50
  private val pendingDownloads = mutableSetOf<Int>()
  private val pendingCancellations = mutableSetOf<Int>()
  private val pendingDownloadRequests = mutableMapOf<Int, String>()
  private val pendingCancelRequests = mutableMapOf<Int, String>()
  private val requestSequence = AtomicLong(0)

  private val mutableState = MutableStateFlow(DiagnosticsState(dataSource = DataSourceMode.REAL))
  override val state: StateFlow<DiagnosticsState> = mutableState.asStateFlow()
  private val mutableAuthorization = MutableStateFlow(AuthorizationSession())
  override val authorization: StateFlow<AuthorizationSession> = mutableAuthorization.asStateFlow()
  private val mutableLibrary = MutableStateFlow<LibraryState>(LibraryState.Idle)
  override val library: StateFlow<LibraryState> = mutableLibrary.asStateFlow()

  override fun start() {
    synchronized(lock) {
      if (lifecycle != GatewayLifecycle.NEW) return
      transitionLocked(GatewayLifecycle.STARTING)
      worker = scope.launch { initializeAndReceive() }
    }
  }

  private suspend fun initializeAndReceive() {
    try {
      libraryLoader.load()
      synchronized(lock) {
        if (lifecycle != GatewayLifecycle.STARTING) return
        mutableState.value = mutableState.value.copy(nativeLibraryLoaded = true)
        clientId = native.createClientId()
        countedClient = true
        mutableState.value = mutableState.value.copy(clientCreated = true, clientInstanceCount = instances.incrementAndGet())
        transitionLocked(GatewayLifecycle.RUNNING)
      }
      sendRequest("verbosity", buildJsonObject { put("@type", "setLogVerbosityLevel"); put("new_verbosity_level", 0) })
      sendRequest("startup", buildJsonObject { put("@type", "getAuthorizationState") })
      while (currentCoroutineContext().isActive) {
        val response = native.receive(0.25)
        if (response == null) {
          delay(1)
          continue
        }
        handleResponse(response)
      }
    } catch (error: Throwable) {
      synchronized(lock) {
        if (lifecycle != GatewayLifecycle.CLOSING && lifecycle != GatewayLifecycle.CLOSED) {
          transitionLocked(GatewayLifecycle.FAILED, safeMessage(error))
          mutableAuthorization.value = AuthorizationSession(
            state = AuthorizationState.Other("failed"),
            safeError = safeMessage(error),
          )
        }
      }
    }
  }

  private fun handleResponse(raw: String) {
    val root = runCatching { json.parseToJsonElement(raw).jsonObject }.getOrElse { return }
    val type = root.string("@type") ?: return
    when (type) {
      "updateAuthorizationState" -> handleAuthorization(root.obj("authorization_state") ?: return)
      "authorizationStateWaitTdlibParameters", "authorizationStateWaitPhoneNumber",
      "authorizationStateWaitCode", "authorizationStateWaitPassword", "authorizationStateWaitEmailAddress",
      "authorizationStateWaitEmailCode", "authorizationStateWaitOtherDeviceConfirmation", "authorizationStateReady",
      "authorizationStateLoggingOut", "authorizationStateClosing", "authorizationStateClosed" -> handleAuthorization(root)
      "updateFile" -> root.obj("file")?.let(::handleFile)
      "file" -> {
        val fileId = root.int("id")
        if (synchronized(lock) { pendingDownloadRequests[fileId] == root.string("@extra") }) handleFile(root)
      }
      "user" -> if (synchronized(lock) { pendingLibraryRequest == root.string("@extra") }) requestSavedHistory(root.long("id"))
      "messages" -> if (synchronized(lock) { pendingLibraryRequest == root.string("@extra") }) {
        synchronized(lock) { pendingLibraryRequest = null }
        handleMessages(root)
      }
      "ok" -> handleOk(root)
      "error" -> handleError(root)
    }
  }

  private fun handleAuthorization(value: JsonObject) {
    val mapped = TdLibStateMapper.authorizationState(value.toString()) ?: AuthorizationState.Other(value.string("@type") ?: "unknown")
    synchronized(lock) {
      pendingAuthAction = false
      pendingAuthRequest = null
      mutableAuthorization.value = AuthorizationSession(state = mapped)
      mutableState.value = mutableState.value.copy(authorizationState = mapped)
    }
    when (mapped) {
      AuthorizationState.WaitingForTdlibParameters -> sendTdlibParametersOrExposeMissingConfig()
      AuthorizationState.Ready -> Unit
      AuthorizationState.Closed -> close()
      else -> Unit
    }
  }

  private fun sendTdlibParametersOrExposeMissingConfig() {
    if (!configuration.configured) {
      mutableAuthorization.value = AuthorizationSession(state = AuthorizationState.MissingConfiguration)
      return
    }
    databaseDirectory.mkdirs()
    filesDirectory.mkdirs()
    val request = requestEnvelope("parameters", buildJsonObject {
      put("@type", "setTdlibParameters")
      put("use_test_dc", false)
      put("database_directory", databaseDirectory.absolutePath)
      put("files_directory", filesDirectory.absolutePath)
      put("database_encryption_key", "")
      put("use_file_database", true)
      put("use_chat_info_database", true)
      put("use_message_database", true)
      put("use_secret_chats", false)
      put("api_id", configuration.apiId)
      put("api_hash", configuration.apiHash)
      put("system_language_code", "en")
      put("device_model", "Android")
      put("system_version", android.os.Build.VERSION.RELEASE)
      put("application_version", "1.0")
    })
    synchronized(lock) { pendingParametersRequest = request.string("@extra") }
    send(request)
  }

  override fun submit(action: AuthorizationAction): ActionResult {
    val request = synchronized(lock) {
      if (!configuration.configured && action !is AuthorizationAction.Reset) return ActionResult.MISSING_CONFIGURATION
      if (pendingAuthAction) return ActionResult.DUPLICATE
      val current = mutableAuthorization.value.state
      val built = when (action) {
        is AuthorizationAction.SubmitPhone -> if (current == AuthorizationState.WaitingForPhoneNumber) buildJsonObject {
          put("@type", "setAuthenticationPhoneNumber")
          put("phone_number", action.phone)
          put("settings", JsonNull)
        } else null
        is AuthorizationAction.SubmitCode -> if (current == AuthorizationState.WaitingForCode) request("checkAuthenticationCode", "code" to action.code) else null
        is AuthorizationAction.SubmitPassword -> if (current is AuthorizationState.WaitingForPassword) request("checkAuthenticationPassword", "password" to action.password) else null
        is AuthorizationAction.SubmitEmailAddress -> if (current == AuthorizationState.WaitingForEmailAddress) request("setAuthenticationEmailAddress", "email_address" to action.email) else null
        is AuthorizationAction.SubmitEmailCode -> if (current == AuthorizationState.WaitingForEmailCode) request("checkAuthenticationEmailCode", "code" to buildJsonObject { put("@type", "emailAddressAuthenticationCode"); put("code", action.code) }) else null
        AuthorizationAction.Logout -> if (current == AuthorizationState.Ready) buildJsonObject { put("@type", "logOut") } else null
        AuthorizationAction.Reset -> buildJsonObject { put("@type", "destroy") }
      } ?: return ActionResult.INVALID_STATE
      pendingAuthAction = true
      val enveloped = requestEnvelope("auth", built)
      pendingAuthRequest = enveloped.string("@extra")
      mutableAuthorization.value = mutableAuthorization.value.copy(actionPending = true, safeError = null)
      enveloped
    }
    send(request)
    return ActionResult.ACCEPTED
  }

  override fun loadSavedMessages(limit: Int): ActionResult {
    if (mutableAuthorization.value.state != AuthorizationState.Ready) return ActionResult.INVALID_STATE
    if (mutableLibrary.value == LibraryState.Loading) return ActionResult.DUPLICATE
    pendingHistoryLimit = limit.coerceIn(1, 50)
    mutableLibrary.value = LibraryState.Loading
    val request = requestEnvelope("getMe", buildJsonObject { put("@type", "getMe") })
    synchronized(lock) { pendingLibraryRequest = request.string("@extra") }
    send(request)
    return ActionResult.ACCEPTED
  }

  private fun requestSavedHistory(selfId: Long) {
    val request = requestEnvelope("history", buildJsonObject {
      put("@type", "getChatHistory")
      put("chat_id", selfId)
      put("from_message_id", 0)
      put("offset", 0)
      put("limit", pendingHistoryLimit)
      put("only_local", false)
    })
    synchronized(lock) { pendingLibraryRequest = request.string("@extra") }
    send(request)
  }

  private fun handleMessages(root: JsonObject) {
    val items = root["messages"]?.jsonArray.orEmpty().mapNotNull(::mapMessage).distinctBy { it.fileId }
    mutableLibrary.value = if (items.isEmpty()) LibraryState.Empty else LibraryState.Content(items)
  }

  private fun mapMessage(element: JsonElement): MediaItem? {
    val message = element.jsonObject
    val content = message.obj("content") ?: return null
    val messageId = message.long("id")
    val chatId = message.long("chat_id")
    val type = content.string("@type")
    val media: JsonObject
    val file: JsonObject
    val kind: MediaKind
    val name: String
    val duration: Int
    when (type) {
      "messagePhoto" -> {
        media = content.obj("photo") ?: return null
        val size = media["sizes"]?.jsonArray?.lastOrNull()?.jsonObject ?: return null
        file = size.obj("photo") ?: return null
        kind = MediaKind.IMAGE; name = "photo-$messageId.jpg"; duration = 0
      }
      "messageVideo" -> {
        media = content.obj("video") ?: return null; file = media.obj("video") ?: return null
        kind = MediaKind.VIDEO; name = media.string("file_name").orEmpty().ifBlank { "video-$messageId.mp4" }; duration = media.int("duration")
      }
      "messageAnimation" -> {
        media = content.obj("animation") ?: return null; file = media.obj("animation") ?: return null
        kind = MediaKind.ANIMATION; name = media.string("file_name").orEmpty().ifBlank { "animation-$messageId" }; duration = media.int("duration")
      }
      "messageDocument" -> {
        media = content.obj("document") ?: return null; file = media.obj("document") ?: return null
        kind = MediaKind.DOCUMENT; name = media.string("file_name").orEmpty().ifBlank { "document-$messageId" }; duration = 0
      }
      else -> return null
    }
    val local = file.obj("local")
    val path = local?.string("path")?.takeIf { it.isNotBlank() }
    val complete = local?.bool("is_downloading_completed") == true && path != null && File(path).isFile
    return MediaItem(
      id = messageId,
      sourceId = chatId,
      name = name,
      kind = kind,
      downloadState = if (complete) DownloadState.Complete else DownloadState.NotDownloaded,
      fileId = file.int("id"),
      sizeBytes = file.long("size"),
      durationSeconds = duration,
      localPath = path.takeIf { complete },
    )
  }

  internal fun mapMessageForTest(raw: String): MediaItem? =
    runCatching { mapMessage(json.parseToJsonElement(raw)) }.getOrNull()

  internal fun handleResponseForTest(raw: String) = handleResponse(raw)

  override fun download(fileId: Int): ActionResult {
    val item = (mutableLibrary.value as? LibraryState.Content)?.items?.firstOrNull { it.fileId == fileId }
      ?: return ActionResult.INVALID_STATE
    if (item.downloadState == DownloadState.Complete && item.localPath?.let(::File)?.isFile == true) return ActionResult.DUPLICATE
    val request = synchronized(lock) {
      if (fileId in pendingCancellations || !pendingDownloads.add(fileId)) return ActionResult.DUPLICATE
      requestEnvelope("download-$fileId", buildJsonObject {
        put("@type", "downloadFile")
        put("file_id", fileId)
        put("priority", 16)
        put("synchronous", false)
      }).also { pendingDownloadRequests[fileId] = it.string("@extra").orEmpty() }
    }
    send(request)
    updateItem(fileId) { it.copy(downloadState = DownloadState.Downloading(0)) }
    return ActionResult.ACCEPTED
  }

  override fun cancelDownload(fileId: Int): ActionResult {
    val request = synchronized(lock) {
      if (fileId !in pendingDownloads || !pendingCancellations.add(fileId)) return ActionResult.INVALID_STATE
      requestEnvelope("cancel-$fileId", buildJsonObject {
        put("@type", "cancelDownloadFile")
        put("file_id", fileId)
        put("only_if_pending", false)
      }).also { pendingCancelRequests[fileId] = it.string("@extra").orEmpty() }
    }
    send(request)
    updateItem(fileId) { it.copy(downloadState = DownloadState.Canceled, localPath = null) }
    return ActionResult.ACCEPTED
  }

  private fun handleFile(file: JsonObject) {
    val fileId = file.int("id")
    if (synchronized(lock) { fileId !in pendingDownloads || fileId in pendingCancellations }) return
    val local = file.obj("local") ?: return
    val complete = local.bool("is_downloading_completed")
    val total = file.long("expected_size").takeIf { it > 0 } ?: file.long("size")
    val downloaded = local.long("downloaded_size")
    val percent = if (total > 0) ((downloaded * 100) / total).toInt().coerceIn(0, 100) else 0
    val path = local.string("path").orEmpty()
    if (complete && path.isNotBlank() && File(path).isFile) synchronized(lock) {
      pendingDownloads.remove(fileId)
      pendingDownloadRequests.remove(fileId)
    }
    updateItem(fileId) {
      if (complete && path.isNotBlank() && File(path).isFile) it.copy(downloadState = DownloadState.Complete, localPath = path)
      else it.copy(downloadState = DownloadState.Downloading(percent))
    }
  }

  override fun preview(itemId: Long): PreviewTarget? {
    val item = (mutableLibrary.value as? LibraryState.Content)?.items?.firstOrNull { it.id == itemId } ?: return null
    val path = item.localPath?.takeIf { File(it).isFile } ?: return null
    return when (item.kind) {
      MediaKind.IMAGE -> PreviewTarget.Image(item.id, path)
      MediaKind.VIDEO -> PreviewTarget.Video(item.id, path)
      else -> null
    }
  }

  private fun updateItem(fileId: Int, transform: (MediaItem) -> MediaItem) {
    val content = mutableLibrary.value as? LibraryState.Content ?: return
    mutableLibrary.value = LibraryState.Content(content.items.map { if (it.fileId == fileId) transform(it) else it })
  }

  private fun handleOk(root: JsonObject) {
    val request = root.string("@extra").orEmpty().substringBefore(':')
    if (!request.startsWith("cancel-")) return
    request.removePrefix("cancel-").toIntOrNull()?.let { fileId ->
      synchronized(lock) {
        if (pendingCancelRequests[fileId] != root.string("@extra")) return
        pendingCancelRequests.remove(fileId)
        pendingDownloadRequests.remove(fileId)
        pendingCancellations.remove(fileId)
        pendingDownloads.remove(fileId)
      }
    }
  }

  private fun handleError(root: JsonObject) {
    val message = SensitiveDataRedactor.redact(root.string("message") ?: "Telegram request failed")
    val extra = root.string("@extra").orEmpty()
    val request = extra.substringBefore(':')
    when {
      request == "auth" && synchronized(lock) { pendingAuthRequest == extra } -> synchronized(lock) {
        pendingAuthAction = false
        pendingAuthRequest = null
        mutableAuthorization.value = mutableAuthorization.value.copy(actionPending = false, safeError = message)
      }
      (request == "getMe" || request == "history") && synchronized(lock) { pendingLibraryRequest == extra } -> {
        synchronized(lock) { pendingLibraryRequest = null }
        mutableLibrary.value = LibraryState.Error(message)
      }
      request == "parameters" && synchronized(lock) { pendingParametersRequest == extra } -> {
        synchronized(lock) { pendingParametersRequest = null }
        mutableAuthorization.value = mutableAuthorization.value.copy(actionPending = false, safeError = message)
      }
      request.startsWith("download-") -> request.removePrefix("download-").toIntOrNull()?.let { fileId ->
        if (synchronized(lock) { pendingDownloadRequests[fileId] != extra }) return
        synchronized(lock) {
          pendingDownloadRequests.remove(fileId)
          pendingDownloads.remove(fileId)
        }
        updateItem(fileId) { it.copy(downloadState = DownloadState.Failed(message), localPath = null) }
      }
      request.startsWith("cancel-") -> request.removePrefix("cancel-").toIntOrNull()?.let { fileId ->
        if (synchronized(lock) { pendingCancelRequests[fileId] != extra }) return
        synchronized(lock) {
          pendingCancelRequests.remove(fileId)
          pendingDownloadRequests.remove(fileId)
          pendingCancellations.remove(fileId)
          pendingDownloads.remove(fileId)
        }
        updateItem(fileId) { it.copy(downloadState = DownloadState.Failed(message), localPath = null) }
      }
    }
  }

  private fun sendRequest(prefix: String, body: JsonObject) {
    send(requestEnvelope(prefix, body))
  }

  private fun requestEnvelope(prefix: String, body: JsonObject): JsonObject =
    JsonObject(body + ("@extra" to JsonPrimitive("$prefix:${requestSequence.incrementAndGet()}")))

  private fun send(body: JsonObject) {
    val id = synchronized(lock) { clientId } ?: return
    native.send(id, body.toString())
  }

  override fun close() {
    val id: Int?
    synchronized(lock) {
      if (lifecycle == GatewayLifecycle.CLOSING || lifecycle == GatewayLifecycle.CLOSED) return
      transitionLocked(GatewayLifecycle.CLOSING)
      mutableAuthorization.value = AuthorizationSession(AuthorizationState.Closing)
      id = clientId; clientId = null
      if (countedClient) { countedClient = false; instances.updateAndGet { if (it > 0) it - 1 else 0 } }
      mutableState.value = mutableState.value.copy(clientCreated = false, clientInstanceCount = instances.get())
      worker?.cancel(); worker = null
      pendingDownloads.clear()
      pendingCancellations.clear()
      pendingDownloadRequests.clear()
      pendingCancelRequests.clear()
      pendingAuthRequest = null
      pendingParametersRequest = null
      pendingLibraryRequest = null
    }
    scope.launch {
      if (id != null) runCatching { native.send(id, "{\"@type\":\"close\"}") }
      synchronized(lock) {
        transitionLocked(GatewayLifecycle.CLOSED)
        mutableAuthorization.value = AuthorizationSession(AuthorizationState.Closed)
      }
      scope.cancel()
    }
  }

  private fun transitionLocked(next: GatewayLifecycle, error: String? = null) {
    lifecycle = next
    mutableState.value = mutableState.value.copy(lifecycle = next, safeError = error)
  }

  private fun safeMessage(error: Throwable) = SensitiveDataRedactor.redact("${error::class.java.simpleName}: TDLib initialization failed")

  internal companion object {
    private val instances = AtomicInteger(0)
    fun activeClientCountForTest(): Int = instances.get()
    fun resetClientCountForTest() = instances.set(0)
  }
}

private fun request(type: String, pair: Pair<String, Any>): JsonObject = buildJsonObject {
  put("@type", type)
  when (val value = pair.second) {
    is String -> put(pair.first, value)
    is JsonElement -> put(pair.first, value)
  }
}

private fun JsonObject.string(name: String) = this[name]?.jsonPrimitive?.contentOrNull
private fun JsonObject.int(name: String) = this[name]?.jsonPrimitive?.intOrNull ?: 0
private fun JsonObject.long(name: String) = this[name]?.jsonPrimitive?.longOrNull ?: 0L
private fun JsonObject.bool(name: String) = this[name]?.jsonPrimitive?.booleanOrNull == true
private fun JsonObject.obj(name: String) = this[name] as? JsonObject
