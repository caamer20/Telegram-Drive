package com.nmtuong.telegramdrive

import android.app.Application
import com.nmtuong.telegramdrive.bootstrap.AppContainer

class TelegramDriveApplication : Application() {
  lateinit var container: AppContainer
    private set

  override fun onCreate() {
    super.onCreate()
    container = AppContainer.create(this)
    container.start()
  }
}
