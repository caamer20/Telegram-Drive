package com.nmtuong.telegramdrive.bootstrap

import android.content.Context
import com.nmtuong.telegramdrive.BuildConfig
import com.nmtuong.telegramdrive.data.FakeTelegramRepository
import com.nmtuong.telegramdrive.data.RealTelegramRepository
import com.nmtuong.telegramdrive.data.TelegramRepository
import com.nmtuong.telegramdrive.data.fake.FakeTelegramCatalog
import com.nmtuong.telegramdrive.domain.DataSourceMode
import com.nmtuong.telegramdrive.domain.ActionResult
import com.nmtuong.telegramdrive.domain.AuthorizationAction
import com.nmtuong.telegramdrive.telegram.TdLibJsonGateway
import com.nmtuong.telegramdrive.security.TelegramApiConfiguration
import java.io.Closeable

class AppContainer private constructor(
  val telegramRepository: TelegramRepository,
  val sampleCatalog: FakeTelegramCatalog,
) : Closeable {
  fun start() = telegramRepository.start()
  fun logout(): ActionResult = telegramRepository.submit(AuthorizationAction.Logout)
  fun resetAccount(): ActionResult = telegramRepository.submit(AuthorizationAction.Reset)
  override fun close() = telegramRepository.close()

  companion object {
    fun create(context: Context): AppContainer {
      val catalog = FakeTelegramCatalog.stable()
      val repository = if (BuildConfig.TELEGRAM_DATA_SOURCE == DataSourceMode.FAKE.id) {
        FakeTelegramRepository(
          catalog,
          context.cacheDir.resolve("fake-media"),
          videoBytes = { context.assets.open("fake-video.mp4").use { it.readBytes() } },
        )
      } else {
        RealTelegramRepository(
          TdLibJsonGateway(
            context.applicationContext,
            TelegramApiConfiguration(BuildConfig.TELEGRAM_API_ID, BuildConfig.TELEGRAM_API_HASH),
          ),
        )
      }
      return AppContainer(repository, catalog)
    }
  }
}
