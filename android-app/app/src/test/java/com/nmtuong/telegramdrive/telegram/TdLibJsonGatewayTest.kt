package com.nmtuong.telegramdrive.telegram

import com.nmtuong.telegramdrive.domain.*
import com.nmtuong.telegramdrive.security.TelegramApiConfiguration
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class TdLibJsonGatewayTest {
  @After fun resetCounter() = TdLibJsonGateway.resetClientCountForTest()

  @Test fun startTwiceCreatesOneNativeClient() = withGateway { gateway, native ->
    gateway.start()
    gateway.start()
    runCurrent()
    assertEquals(1, native.createCalls)
    assertEquals(true, native.requests.first().contains("setLogVerbosityLevel"))
    assertEquals(true, native.requests.first().contains("new_verbosity_level\":0"))
    assertEquals(1, TdLibJsonGateway.activeClientCountForTest())
    gateway.close()
    runCurrent()
  }

  @Test fun closeTwiceIsIdempotentAndCounterNeverBecomesNegative() = withGateway { gateway, native ->
    gateway.start()
    runCurrent()
    gateway.close()
    gateway.close()
    runCurrent()
    assertEquals(1, native.closeRequests)
    assertEquals(0, TdLibJsonGateway.activeClientCountForTest())
    assertEquals(GatewayLifecycle.CLOSED, gateway.state.value.lifecycle)
  }

  @Test fun closeDuringLibraryInitializationPreventsClientCreation() {
    runTest {
      val dispatcher = StandardTestDispatcher(testScheduler)
      val native = RecordingNative()
      lateinit var gateway: TdLibJsonGateway
      gateway = TdLibJsonGateway(
        native = native,
        libraryLoader = NativeLibraryLoader { gateway.close() },
        dispatcher = dispatcher,
      )
      gateway.start()
      runCurrent()
      assertEquals(0, native.createCalls)
      assertEquals(0, TdLibJsonGateway.activeClientCountForTest())
      assertEquals(GatewayLifecycle.CLOSED, gateway.state.value.lifecycle)
    }
  }

  @Test fun closeAfterLibraryLoadFailureIsSafe() {
    runTest {
      val dispatcher = StandardTestDispatcher(testScheduler)
      val gateway = TdLibJsonGateway(
        native = RecordingNative(),
        libraryLoader = NativeLibraryLoader { error("load failed") },
        dispatcher = dispatcher,
      )
      gateway.start()
      runCurrent()
      assertEquals(GatewayLifecycle.FAILED, gateway.state.value.lifecycle)
      gateway.close()
      runCurrent()
      assertEquals(GatewayLifecycle.CLOSED, gateway.state.value.lifecycle)
      assertEquals(0, TdLibJsonGateway.activeClientCountForTest())
    }
  }

  @Test fun closeAfterCreateFailureIsSafe() {
    runTest {
      val dispatcher = StandardTestDispatcher(testScheduler)
      val native = RecordingNative(failCreate = true)
      val gateway = TdLibJsonGateway(
        native = native,
        libraryLoader = NativeLibraryLoader {},
        dispatcher = dispatcher,
      )
      gateway.start()
      runCurrent()
      assertEquals(GatewayLifecycle.FAILED, gateway.state.value.lifecycle)
      assertFalse(gateway.state.value.clientCreated)
      gateway.close()
      runCurrent()
      assertEquals(0, TdLibJsonGateway.activeClientCountForTest())
    }
  }

  @Test fun immediateStartThenCloseDoesNotCreateClient() = withGateway { gateway, native ->
    gateway.start()
    gateway.close()
    runCurrent()
    assertEquals(0, native.createCalls)
    assertEquals(GatewayLifecycle.CLOSED, gateway.state.value.lifecycle)
  }

  @Test fun mapsSupportedSavedMessageAndFiltersUnsupportedContent() = withGateway { gateway, _ ->
    val photo = """{"id":42,"chat_id":7,"content":{"@type":"messagePhoto","photo":{"sizes":[{"photo":{"id":99,"size":123,"local":{"path":"","is_downloading_completed":false}}}]}}}"""
    val text = """{"id":43,"chat_id":7,"content":{"@type":"messageText","text":{"text":"ignored"}}}"""
    val mapped = gateway.mapMessageForTest(photo)
    assertEquals(42L, mapped?.id)
    assertEquals(99, mapped?.fileId)
    assertEquals(MediaKind.IMAGE, mapped?.kind)
    assertNull(gateway.mapMessageForTest(text))
  }

  @Test fun cancelAcknowledgementBlocksRetryAndIgnoresLateFileUpdate() = runTest {
    val native = RecordingNative()
    val gateway = TdLibJsonGateway(
      configuration = TelegramApiConfiguration(1, "configured-placeholder"),
      native = native,
      libraryLoader = NativeLibraryLoader {},
      dispatcher = StandardTestDispatcher(testScheduler),
    )
    gateway.start()
    runCurrent()
    gateway.handleResponseForTest("""{"@type":"authorizationStateReady"}""")
    assertEquals(ActionResult.ACCEPTED, gateway.loadSavedMessages(50))
    val getMeExtra = requestExtra(native.requests.last())
    gateway.handleResponseForTest("""{"@type":"user","@extra":"$getMeExtra","id":7}""")
    val historyExtra = requestExtra(native.requests.last())
    gateway.handleResponseForTest(
      """{"@type":"messages","@extra":"$historyExtra","messages":[{"id":42,"chat_id":7,"content":{"@type":"messagePhoto","photo":{"sizes":[{"photo":{"id":99,"size":123,"local":{"path":"","is_downloading_completed":false}}}]}}}]}""",
    )
    assertEquals(ActionResult.ACCEPTED, gateway.download(99))
    assertEquals(ActionResult.ACCEPTED, gateway.cancelDownload(99))
    val cancelExtra = requestExtra(native.requests.last())
    assertEquals(ActionResult.DUPLICATE, gateway.download(99))
    gateway.handleResponseForTest(
      """{"@type":"updateFile","file":{"id":99,"size":123,"local":{"path":"","downloaded_size":80,"is_downloading_completed":false}}}""",
    )
    val canceled = (gateway.library.value as LibraryState.Content).items.single()
    assertEquals(DownloadState.Canceled, canceled.downloadState)
    gateway.handleResponseForTest("""{"@type":"ok","@extra":"$cancelExtra"}""")
    assertEquals(ActionResult.ACCEPTED, gateway.download(99))
    gateway.close()
    runCurrent()
  }

  @Test fun authSubmitIsStateGatedDeduplicatedAndErrorIsRedacted() = runTest {
    val native = RecordingNative()
    val gateway = TdLibJsonGateway(
      configuration = TelegramApiConfiguration(1, "configured-placeholder"),
      native = native,
      libraryLoader = NativeLibraryLoader {},
      dispatcher = StandardTestDispatcher(testScheduler),
    )
    gateway.start()
    runCurrent()
    gateway.handleResponseForTest("""{"@type":"authorizationStateWaitPhoneNumber"}""")
    assertEquals(ActionResult.INVALID_STATE, gateway.submit(AuthorizationAction.SubmitCode("input")))
    assertEquals(ActionResult.ACCEPTED, gateway.submit(AuthorizationAction.SubmitPhone("+000000000")))
    assertEquals(ActionResult.DUPLICATE, gateway.submit(AuthorizationAction.SubmitPhone("+000000000")))
    assertEquals(true, native.requests.last().contains("\"settings\":null"))
    gateway.handleResponseForTest(
      """{"@type":"error","@extra":"auth:stale","code":400,"message":"phone_number=+000000000"}""",
    )
    assertEquals(true, gateway.authorization.value.actionPending)
    val authExtra = requestExtra(native.requests.last())
    gateway.handleResponseForTest(
      """{"@type":"error","@extra":"$authExtra","code":400,"message":"phone_number=+000000000 code=value-c"}""",
    )
    assertFalse(gateway.authorization.value.actionPending)
    assertFalse(gateway.authorization.value.safeError.orEmpty().contains("000000000"))
    assertFalse(gateway.authorization.value.safeError.orEmpty().contains("value-c"))
    gateway.close()
    runCurrent()
  }

  private fun withGateway(
    block: suspend TestScope.(TdLibJsonGateway, RecordingNative) -> Unit,
  ) = runTest {
    val dispatcher = StandardTestDispatcher(testScheduler)
    val native = RecordingNative()
    val gateway = TdLibJsonGateway(native = native, libraryLoader = NativeLibraryLoader {}, dispatcher = dispatcher)
    block(gateway, native)
  }

  private fun requestExtra(request: String): String =
    Regex("""\"@extra\":\"([^\"]+)\"""").find(request)!!.groupValues[1]
}

private class RecordingNative(private val failCreate: Boolean = false) : TdLibNative {
  var createCalls = 0
  var closeRequests = 0
  val requests = mutableListOf<String>()
  private var responseDelivered = false

  override fun createClientId(): Int {
    createCalls++
    if (failCreate) error("create failed")
    return 7
  }

  override fun send(clientId: Int, request: String) {
    requests += request
    if (request.contains("\"close\"")) closeRequests++
  }

  override fun receive(timeoutSeconds: Double): String? {
    if (responseDelivered) return null
    responseDelivered = true
    return "{\"@type\":\"authorizationStateWaitTdlibParameters\"}"
  }
}
