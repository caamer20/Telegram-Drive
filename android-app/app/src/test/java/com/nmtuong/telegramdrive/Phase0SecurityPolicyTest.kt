package com.nmtuong.telegramdrive

import java.io.File
import java.security.MessageDigest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class Phase0SecurityPolicyTest {
  private val repositoryRoot: File by lazy {
    generateSequence(File(requireNotNull(System.getProperty("user.dir"))).canonicalFile) { it.parentFile }
      .first { File(it, "android-app/app/src/main/AndroidManifest.xml").isFile }
  }

  @Test fun backupAndDeviceTransferAreDisabledAndDefenseInDepthRulesRemain() {
    val manifest = file("android-app/app/src/main/AndroidManifest.xml").readText()
    assertTrue(manifest.contains("android:allowBackup=\"false\""))
    assertTrue(manifest.contains("android:fullBackupContent=\"@xml/backup_rules\""))
    assertTrue(manifest.contains("android:dataExtractionRules=\"@xml/data_extraction_rules\""))

    val legacyRules = file("android-app/app/src/main/res/xml/backup_rules.xml").readText()
    val modernRules = file("android-app/app/src/main/res/xml/data_extraction_rules.xml").readText()
    listOf("root", "file", "database", "sharedpref", "external").forEach { domain ->
      assertTrue(legacyRules.contains("domain=\"$domain\" path=\".\""))
      assertTrue(modernRules.contains("domain=\"$domain\" path=\".\""))
    }
    assertTrue(modernRules.contains("<cloud-backup>"))
    assertTrue(modernRules.contains("<device-transfer>"))
  }

  @Test fun pinnedBuildAndPackagedBinariesUseSupportedCrypto() {
    val retiredVersion = listOf("OpenSSL", "_1_1_1", "w").joinToString("")
    val retiredBinaryVersion = listOf("OpenSSL ", "1.1.1", "w").joinToString("")
    val script = file("scripts/build-tdlib-android.sh").readText()
    assertFalse(script.contains(retiredVersion))
    assertTrue(script.contains("OPENSSL_VERSION=\"3.5.7\""))
    assertTrue(script.contains("OPENSSL_SHA256="))

    val metadata = file("android-app/tdlib-build-metadata.txt").readLines()
      .filter { it.contains('=') }
      .associate { it.substringBefore('=') to it.substringAfter('=') }
    assertEquals("3.5.7", metadata["openssl_version"])
    listOf("arm64-v8a", "x86_64").forEach { abi ->
      val binary = file("android-app/${metadata.getValue("binary.$abi.path")}")
      assertTrue(binary.isFile)
      assertEquals(metadata["binary.$abi.sha256"], binary.sha256())
      val printableBinary = binary.readBytes().toString(Charsets.ISO_8859_1)
      assertTrue(printableBinary.contains("OpenSSL 3.5.7"))
      assertFalse(printableBinary.contains(retiredBinaryVersion))
    }
  }

  private fun file(path: String) = File(repositoryRoot, path)
}

private fun File.sha256(): String {
  val digest = MessageDigest.getInstance("SHA-256")
  inputStream().use { input ->
    val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
    while (true) {
      val count = input.read(buffer)
      if (count < 0) break
      digest.update(buffer, 0, count)
    }
  }
  return digest.digest().joinToString("") { "%02x".format(it) }
}
