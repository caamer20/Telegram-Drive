package com.nmtuong.telegramdrive.feature.preview

class VideoPlayerReleaseGuard(private val releasePlayer: () -> Unit) {
  private var released = false

  fun release() {
    if (released) return
    released = true
    releasePlayer()
  }
}
