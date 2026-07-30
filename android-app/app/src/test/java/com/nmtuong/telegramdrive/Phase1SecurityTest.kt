package com.nmtuong.telegramdrive

import com.nmtuong.telegramdrive.security.SensitiveDataRedactor
import com.nmtuong.telegramdrive.security.TelegramApiConfiguration
import org.junit.Assert.*
import org.junit.Test

class Phase1SecurityTest {
  @Test fun configurationRequiresBothValues() {
    assertFalse(TelegramApiConfiguration(0, "").configured)
    assertFalse(TelegramApiConfiguration(123, "").configured)
    assertTrue(TelegramApiConfiguration(123, "local-secret").configured)
  }

  @Test fun redactorRemovesCredentialsAndPhoneNumbers() {
    val redacted = SensitiveDataRedactor.redact(
      """phone_number=+000000000 password=value-a {"api_hash":"value-b","code":"value-c"} tg://login?token=value-d""",
    )
    listOf("000000000", "value-a", "value-b", "value-c", "value-d").forEach {
      assertFalse(redacted.contains(it))
    }
    assertTrue(redacted.contains("[REDACTED]"))
  }
}
