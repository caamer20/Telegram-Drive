package com.nmtuong.telegramdrive.telegram

import org.drinkless.tdlib.JsonClient

internal fun interface NativeLibraryLoader {
  fun load()
}

internal interface TdLibNative {
  fun createClientId(): Int
  fun send(clientId: Int, request: String)
  fun receive(timeoutSeconds: Double): String?
}

internal object JsonTdLibNative : TdLibNative {
  override fun createClientId(): Int = JsonClient.createClientId()
  override fun send(clientId: Int, request: String) = JsonClient.send(clientId, request)
  override fun receive(timeoutSeconds: Double): String? = JsonClient.receive(timeoutSeconds)
}
