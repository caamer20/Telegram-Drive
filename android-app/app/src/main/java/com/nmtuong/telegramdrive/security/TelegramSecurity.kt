package com.nmtuong.telegramdrive.security

data class TelegramApiConfiguration(val apiId: Int, val apiHash: String) {
  val configured: Boolean get() = apiId > 0 && apiHash.isNotBlank()
}

object SensitiveDataRedactor {
  private val phone = Regex("(?<!\\d)\\+?\\d[\\d -]{6,}\\d")
  private val jsonField = Regex("(?i)(\\\"(?:api_hash|code|password|phone_number)\\\"\\s*:\\s*\\\")[^\\\"]*(\\\")")
  private val plainField = Regex("(?i)\\b(api_hash|code|password|phone_number)\\s*=\\s*[^,}\\s]+")
  private val queryField = Regex("(?i)([?&](?:token|code|password|phone_number)=)[^&\\s]+")

  fun redact(value: String): String = value
    .replace(jsonField, "$1[REDACTED]$2")
    .replace(plainField) { "${it.groupValues[1]}=[REDACTED]" }
    .replace(queryField, "$1[REDACTED]")
    .replace(phone, "[REDACTED]")
    .take(240)
}
