package com.nmtuong.telegramdrive

import com.nmtuong.telegramdrive.data.FakeTelegramRepository
import com.nmtuong.telegramdrive.data.fake.FakeTelegramCatalog
import com.nmtuong.telegramdrive.domain.*
import java.io.File
import java.nio.file.Files
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class FakePhase1FlowTest {
  @Test fun authorizationRejectsInvalidActionsAndRunsDeterministicFlow() {
    val repository = repository()
    assertEquals(ActionResult.INVALID_STATE, repository.submit(AuthorizationAction.SubmitCode("12345")))
    assertEquals(ActionResult.ACCEPTED, repository.submit(AuthorizationAction.SubmitPhone("+100000000")))
    assertEquals(ActionResult.INVALID_STATE, repository.submit(AuthorizationAction.SubmitPhone("+100000000")))
    assertEquals(ActionResult.ACCEPTED, repository.submit(AuthorizationAction.SubmitCode("12345")))
    assertTrue(repository.authorization.value.state is AuthorizationState.WaitingForPassword)
    assertEquals(ActionResult.ACCEPTED, repository.submit(AuthorizationAction.SubmitPassword("secret")))
    assertEquals(AuthorizationState.Ready, repository.authorization.value.state)
  }

  @Test fun savedMessagesAreBoundedUniqueAndPreviewRequiresLocalFile() = runTest {
    val repository = repository(StandardTestDispatcher(testScheduler))
    authorize(repository)
    assertEquals(ActionResult.ACCEPTED, repository.loadSavedMessages(2))
    val items = (repository.library.value as LibraryState.Content).items
    assertEquals(2, items.size)
    assertEquals(items.size, items.distinctBy { it.fileId }.size)
    assertTrue(items.all { it.kind in setOf(MediaKind.IMAGE, MediaKind.VIDEO, MediaKind.ANIMATION, MediaKind.DOCUMENT) })
    val image = items.first { it.kind == MediaKind.IMAGE }
    assertNull(repository.preview(image.id))
    assertEquals(ActionResult.ACCEPTED, repository.download(image.fileId))
    assertEquals(ActionResult.DUPLICATE, repository.download(image.fileId))
    advanceUntilIdle()
    assertTrue(repository.preview(image.id) is PreviewTarget.Image)
    val preview = repository.preview(image.id) as PreviewTarget.Image
    assertTrue(File(preview.path).delete())
    assertNull(repository.preview(image.id))
    assertEquals(ActionResult.ACCEPTED, repository.download(image.fileId))
    advanceUntilIdle()
    assertTrue(repository.preview(image.id) is PreviewTarget.Image)
    assertEquals(ActionResult.INVALID_STATE, repository.cancelDownload(image.fileId))
    repository.close()
  }

  @Test fun fakeDownloadCoversProgressCancelFailureAndRetryGeneration() = runTest {
    val repository = repository(StandardTestDispatcher(testScheduler), stepDelayMillis = 100)
    authorize(repository)
    repository.loadSavedMessages(3)
    val initial = (repository.library.value as LibraryState.Content).items
    val image = initial.first { it.kind == MediaKind.IMAGE }
    val animation = initial.first { it.kind == MediaKind.ANIMATION }

    assertEquals(ActionResult.ACCEPTED, repository.download(image.fileId))
    assertEquals(DownloadState.Downloading(0), item(repository, image.fileId).downloadState)
    advanceTimeBy(100)
    runCurrent()
    assertEquals(DownloadState.Downloading(50), item(repository, image.fileId).downloadState)
    assertEquals(ActionResult.ACCEPTED, repository.cancelDownload(image.fileId))
    advanceUntilIdle()
    assertEquals(DownloadState.Canceled, item(repository, image.fileId).downloadState)

    assertEquals(ActionResult.ACCEPTED, repository.download(image.fileId))
    advanceUntilIdle()
    assertEquals(DownloadState.Complete, item(repository, image.fileId).downloadState)

    assertEquals(ActionResult.ACCEPTED, repository.download(animation.fileId))
    advanceUntilIdle()
    assertTrue(item(repository, animation.fileId).downloadState is DownloadState.Failed)
    repository.close()
  }

  @Test fun logoutClearsLibraryState() {
    val repository = repository()
    authorize(repository)
    repository.loadSavedMessages()
    assertEquals(ActionResult.ACCEPTED, repository.submit(AuthorizationAction.Logout))
    assertEquals(AuthorizationState.Closed, repository.authorization.value.state)
    assertEquals(LibraryState.Idle, repository.library.value)
  }

  private fun repository(
    dispatcher: CoroutineDispatcher? = null,
    stepDelayMillis: Long = 1,
  ) = FakeTelegramRepository(
    FakeTelegramCatalog.stable(),
    Files.createTempDirectory("p1-fake").toFile(),
    dispatcher = dispatcher ?: StandardTestDispatcher(),
    downloadStepDelayMillis = stepDelayMillis,
  )

  private fun item(repository: FakeTelegramRepository, fileId: Int): MediaItem =
    (repository.library.value as LibraryState.Content).items.first { it.fileId == fileId }
  private fun authorize(repository: FakeTelegramRepository) {
    repository.submit(AuthorizationAction.SubmitPhone("+100000000"))
    repository.submit(AuthorizationAction.SubmitCode("12345"))
    repository.submit(AuthorizationAction.SubmitPassword("secret"))
  }
}
