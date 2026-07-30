package com.nmtuong.telegramdrive.navigation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.nmtuong.telegramdrive.bootstrap.AppContainer
import com.nmtuong.telegramdrive.domain.AuthorizationState
import com.nmtuong.telegramdrive.domain.PreviewTarget
import com.nmtuong.telegramdrive.feature.auth.AuthorizationScreen
import com.nmtuong.telegramdrive.feature.auth.AuthorizationViewModel
import com.nmtuong.telegramdrive.feature.library.LibraryScreen
import com.nmtuong.telegramdrive.feature.library.LibraryViewModel
import com.nmtuong.telegramdrive.feature.preview.ImagePreviewScreen
import com.nmtuong.telegramdrive.feature.preview.VideoPreviewScreen

@Composable
fun AppNavigation(container: AppContainer) {
  val authorizationViewModel: AuthorizationViewModel = viewModel {
    AuthorizationViewModel(container.telegramRepository)
  }
  val libraryViewModel: LibraryViewModel = viewModel {
    LibraryViewModel(container.telegramRepository)
  }
  val authorization by authorizationViewModel.state.collectAsStateWithLifecycle()
  var preview by remember { mutableStateOf<PreviewTarget?>(null) }

  when (val target = preview) {
    is PreviewTarget.Image -> ImagePreviewScreen(target.path) { preview = null }
    is PreviewTarget.Video -> VideoPreviewScreen(target.path) { preview = null }
    null -> if (authorization.state == AuthorizationState.Ready) {
      LibraryScreen(libraryViewModel) { preview = it }
    } else {
      AuthorizationScreen(authorizationViewModel)
    }
  }
}
