package com.nmtuong.telegramdrive.feature.auth

import androidx.lifecycle.ViewModel
import com.nmtuong.telegramdrive.data.TelegramRepository
import com.nmtuong.telegramdrive.domain.AuthorizationAction

class AuthorizationViewModel(private val repository: TelegramRepository) : ViewModel() {
  val state = repository.authorization
  fun submit(action: AuthorizationAction) = repository.submit(action)
}
