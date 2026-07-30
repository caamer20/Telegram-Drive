package com.nmtuong.telegramdrive.telegram

import android.content.Context
import com.nmtuong.telegramdrive.domain.DataSourceMode
import com.nmtuong.telegramdrive.domain.DiagnosticsState
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.drinkless.tdlib.JsonClient

class TdLibJsonGateway(@Suppress("UNUSED_PARAMETER") context: Context) : TdLibGateway {
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
  private val started = AtomicBoolean(false)
  private var clientId: Int? = null
  private val mutableState = MutableStateFlow(DiagnosticsState(dataSource = DataSourceMode.REAL))
  override val state: StateFlow<DiagnosticsState> = mutableState.asStateFlow()

  override fun start() {
    if (!started.compareAndSet(false, true)) return
    scope.launch {
      try {
        System.loadLibrary("tdjsonjava")
        mutableState.value = mutableState.value.copy(nativeLibraryLoaded = true)
        clientId = JsonClient.createClientId()
        val count = instances.incrementAndGet()
        mutableState.value = mutableState.value.copy(clientCreated = true, clientInstanceCount = count)
        JsonClient.send(clientId!!, "{\"@type\":\"getAuthorizationState\",\"@extra\":\"phase-0\"}")
        while (isActive) {
          val response = JsonClient.receive(0.25) ?: continue
          val authorization = TdLibStateMapper.authorizationState(response) ?: continue
          mutableState.value = mutableState.value.copy(authorizationState = authorization)
        }
      } catch (error: Throwable) {
        mutableState.value = mutableState.value.copy(safeError = safeMessage(error))
      }
    }
  }

  override fun close() {
    clientId?.let { id -> runCatching { JsonClient.send(id, "{\"@type\":\"close\"}") } }
    clientId = null
    scope.cancel()
    if (started.getAndSet(false)) instances.decrementAndGet()
  }

  private fun safeMessage(error: Throwable): String =
    "${error::class.java.simpleName}: TDLib initialization failed".take(160)

  companion object { private val instances = AtomicInteger(0) }
}
