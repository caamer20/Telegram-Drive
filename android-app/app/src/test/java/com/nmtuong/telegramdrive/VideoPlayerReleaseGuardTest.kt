package com.nmtuong.telegramdrive

import com.nmtuong.telegramdrive.feature.preview.VideoPlayerReleaseGuard
import org.junit.Assert.assertEquals
import org.junit.Test

class VideoPlayerReleaseGuardTest {
  @Test fun releasesPlayerExactlyOnce() {
    var releases = 0
    val guard = VideoPlayerReleaseGuard { releases++ }
    guard.release()
    guard.release()
    assertEquals(1, releases)
  }
}
