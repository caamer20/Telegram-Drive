package com.nmtuong.telegramdrive

import com.nmtuong.telegramdrive.data.fake.FakeTelegramCatalog
import com.nmtuong.telegramdrive.domain.*
import org.junit.Assert.assertTrue
import org.junit.Test

class FakeTelegramCatalogTest {
  @Test fun stableCatalogCoversPhaseZeroMediaAndDownloadStates() {
    val catalog = FakeTelegramCatalog.stable()
    assertTrue(catalog.sources.any { it.savedMessages })
    assertTrue(catalog.media.map { it.kind }.containsAll(MediaKind.entries))
    assertTrue(catalog.media.any { it.downloadState is DownloadState.Downloading })
    assertTrue(catalog.media.any { it.downloadState is DownloadState.Complete })
    assertTrue(catalog.media.any { it.downloadState is DownloadState.Failed })
  }
}
