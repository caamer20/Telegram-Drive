package com.nmtuong.telegramdrive.data.fake

import com.nmtuong.telegramdrive.domain.*

data class FakeTelegramCatalog(
  val account: Account,
  val sources: List<FileSource>,
  val media: List<MediaItem>,
) {
  companion object {
    fun stable() = FakeTelegramCatalog(
      account = Account(1, "Phase Zero Developer"),
      sources = listOf(
        FileSource(10, "Saved Messages", true),
        FileSource(11, "Design Assets", false),
        FileSource(12, "Project Documents", false),
      ),
      media = listOf(
        MediaItem(100, 10, "mountain.jpg", MediaKind.IMAGE, DownloadState.Complete),
        MediaItem(101, 10, "demo.mp4", MediaKind.VIDEO, DownloadState.Downloading(42)),
        MediaItem(102, 11, "theme.mp3", MediaKind.AUDIO, DownloadState.Complete),
        MediaItem(103, 12, "specification.pdf", MediaKind.PDF, DownloadState.Complete),
        MediaItem(104, 12, "archive.zip", MediaKind.DOCUMENT, DownloadState.Failed("Sample offline failure")),
      ),
    )
  }
}
