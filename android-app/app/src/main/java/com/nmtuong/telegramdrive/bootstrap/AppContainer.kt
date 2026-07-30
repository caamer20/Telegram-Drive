package com.nmtuong.telegramdrive.bootstrap

import android.content.Context
import com.nmtuong.telegramdrive.BuildConfig
import com.nmtuong.telegramdrive.data.FakeTelegramRepository
import com.nmtuong.telegramdrive.data.RealTelegramRepository
import com.nmtuong.telegramdrive.data.TelegramRepository
import com.nmtuong.telegramdrive.data.fake.FakeTelegramCatalog
import com.nmtuong.telegramdrive.domain.DataSourceMode
import com.nmtuong.telegramdrive.telegram.TdLibJsonGateway
import java.io.Closeable

class AppContainer private constructor(
  val telegramRepository: TelegramRepository,
  val sampleCatalog: FakeTelegramCatalog,
) : Closeable {
  fun start() = telegramRepository.start()
  override fun close() = telegramRepository.close()

  companion object {
    fun create(context: Context): AppContainer {
      val catalog = FakeTelegramCatalog.stable()
      val repository = if (BuildConfig.TELEGRAM_DATA_SOURCE == DataSourceMode.FAKE.id) {
        FakeTelegramRepository(catalog)
      } else {
        RealTelegramRepository(TdLibJsonGateway(context.applicationContext))
      }
      return AppContainer(repository, catalog)
    }
  }
}
