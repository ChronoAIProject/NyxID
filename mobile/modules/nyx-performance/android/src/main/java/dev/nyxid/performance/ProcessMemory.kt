package dev.nyxid.performance

import java.io.FileInputStream

/** Process lifetime RSS high-water mark; absent on an unavailable/invalid procfs. */
internal fun parsePeakRssBytes(status: String): Long? {
  val value = Regex("(?m)^VmHWM:\\s+([0-9]+)\\s+kB\\s*$")
    .find(status)?.groupValues?.get(1)?.toLongOrNull() ?: return null
  // Reject impossible values and checked-convert Linux's KiB reporting to bytes.
  return value.takeIf { it in 1..(1L shl 30) }?.times(1024)
}

internal fun readPeakRssBytes(): Long? = runCatching {
  FileInputStream("/proc/self/status").use { stream ->
    val buffer = ByteArray(16 * 1024)
    val read = stream.read(buffer)
    if (read <= 0) null else parsePeakRssBytes(String(buffer, 0, read, Charsets.US_ASCII))
  }
}.getOrNull()
