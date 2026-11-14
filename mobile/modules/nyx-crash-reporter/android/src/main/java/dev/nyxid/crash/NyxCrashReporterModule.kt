package dev.nyxid.crash

import android.os.Build
import android.util.AtomicFile
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import org.json.JSONObject
import org.json.JSONArray
import java.io.File
import java.text.SimpleDateFormat
import java.util.Locale
import java.util.TimeZone
import java.util.UUID

/** Android JS fatal persistence only. No JVM/NDK crash handler is installed. */
class NyxCrashReporterModule : Module() {
  private var armed = false
  private val mutex = Any()
  private val directory: File get() = File(requireNotNull(appContext.reactContext).noBackupFilesDir, "nyx-crash").also { it.mkdirs() }
  private fun bindingFile() = File(directory, "binding.json")
  private fun binding(): JSONObject? = runCatching { JSONObject(bindingFile().readText()).takeIf { v -> v.keys().asSequence().toSet() == setOf("accountId","revision","epochId") && listOf("accountId","revision","epochId").all { UUID.fromString(v.getString(it)) != null } } }.getOrNull()
  private fun atomic(file: File, data: ByteArray) {
    val atomic = AtomicFile(file)
    val stream = atomic.startWrite()
    try { stream.write(data); stream.fd.sync(); atomic.finishWrite(stream) } catch (error: Exception) { atomic.failWrite(stream); throw error }
  }
  private fun records() = directory.listFiles()?.filter { it.name.startsWith("event-") && it.extension == "json" }?.sortedBy { it.name } ?: emptyList()
  private fun purgeInterruptedWrites() {
    directory.listFiles()?.filter { it.isFile && it.name != "binding.json" && !(it.name.startsWith("event-") && it.extension == "json") }?.forEach { it.delete() }
  }
  private fun disarm() { armed = false; directory.listFiles()?.filter { it.isFile }?.forEach { it.delete() } }
  private fun trim() {
    purgeInterruptedWrites()
    val now = System.currentTimeMillis()
    val files = records().filter { file ->
      val valid = runCatching { file.length() <= 32768 && now - JSONObject(file.readText()).getLong("storedAt") in -300000..86400000 }.getOrDefault(false)
      if (!valid) file.delete()
      valid
    }.toMutableList()
    while (files.size > 3 || files.sumOf { it.length() } > 262144) files.removeAt(0).delete()
  }
  private fun sanitize(value: JSONObject): JSONObject? = runCatching {
    val allowed = setOf("schema_version","event_id","captured_at","platform","kind","exception_class","app_version","app_build","artifact_id","os_version","runtime","js_frames","native_images","native_frames")
    require(value.keys().asSequence().all { it in allowed })
    require(value.getInt("schema_version") == 1 && value.getString("kind") == "js_fatal" && value.getString("platform") == "android")
    val event = UUID.fromString(value.getString("event_id")); require(event.version() == 4)
    val captured = value.getString("captured_at"); require(captured.matches(Regex("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}Z$")))
    val formatter = SimpleDateFormat("yyyy-MM-dd\'T\'HH:mm:ss.SSS\'Z\'", Locale.ROOT).apply { timeZone = TimeZone.getTimeZone("UTC"); isLenient = false }
    val timestamp = requireNotNull(formatter.parse(captured)).time; require(System.currentTimeMillis() - timestamp in -300000..86400000)
    require(value.getString("app_version").matches(Regex("^[0-9A-Za-z.+-]{1,32}$")))
    require(value.getString("app_build").matches(Regex("^[0-9A-Za-z.+-]{1,64}$")))
    require(value.getString("os_version").matches(Regex("^[0-9]{1,3}(\\.[0-9]{1,3})?$")))
    require(value.getString("exception_class") in setOf("Error","TypeError","ReferenceError","RangeError","SyntaxError","EvalError","URIError","AggregateError","ChunkLoadError","NonError","Other","Unknown"))
    require(value.getJSONArray("native_images").length() == 0 && value.getJSONArray("native_frames").length() == 0)
    val frames = value.getJSONArray("js_frames"); require(frames.length() <= 64)
    val safeFrames = JSONArray()
    for (i in 0 until frames.length()) {
      val frame = frames.getJSONObject(i); require(frame.keys().asSequence().toSet() == setOf("asset","line","column"))
      require(frame.getString("asset") in setOf("main.jsbundle","index.android.bundle","<native>","<external>"))
      require(frame.getLong("line") in 0..10000000 && frame.getLong("column") in 0..10000000)
      safeFrames.put(JSONObject().put("asset",frame.getString("asset")).put("line",frame.getLong("line")).put("column",frame.getLong("column")))
    }
    val result = JSONObject()
    for (key in listOf("schema_version","event_id","captured_at","platform","kind","exception_class","app_version","app_build","os_version")) result.put(key,value.get(key))
    result.put("js_frames",safeFrames).put("native_images",JSONArray()).put("native_frames",JSONArray())
    if (value.has("artifact_id")) { UUID.fromString(value.getString("artifact_id")); result.put("artifact_id",value.getString("artifact_id")) }
    if (value.has("runtime")) { val runtime = value.getJSONObject("runtime"); require(runtime.keys().asSequence().toSet() == setOf("name","major") && runtime.getString("name") == "hermes" && runtime.getInt("major") in 0..65535);result.put("runtime",JSONObject().put("name","hermes").put("major",runtime.getInt("major"))) }
    result
  }.getOrNull()
  override fun definition() = ModuleDefinition {
    Name("NyxCrashReporter")
    Function("getAppIdentity") {
      val context = requireNotNull(appContext.reactContext)
      val info = context.packageManager.getPackageInfo(context.packageName,0)
      mapOf("osVersion" to Build.VERSION.RELEASE.split(".").take(2).joinToString("."), "version" to (info.versionName ?: "0"), "build" to (if (Build.VERSION.SDK_INT >= 28) info.longVersionCode.toString() else @Suppress("DEPRECATION") info.versionCode.toString()))
    }
    Function("getBinding") { synchronized(mutex) { binding()?.let { value -> listOf("accountId","revision","epochId").associateWith { value.getString(it) } } } }
    Function("arm") { value: Map<String,String> -> synchronized(mutex) {
      require(value.keys == setOf("accountId","revision","epochId")); value.values.forEach { UUID.fromString(it) }
      val json = JSONObject(value)
      val previous = binding()
      if (previous == null || listOf("accountId","revision","epochId").any { previous.getString(it) != value[it] }) disarm()
      atomic(bindingFile(),json.toString().toByteArray())
      armed = true
      true
    } }
    Function("pause") { synchronized(mutex) { armed = false; trim() } }
    Function("disarm") { synchronized(mutex) { disarm() } }
    Function("recordFatalJs") { json: String -> synchronized(mutex) {
      val bound = binding()
      if (armed && bound != null && json.toByteArray().size <= 32768) {
        val safe = runCatching { sanitize(JSONObject(json)) }.getOrNull()
        if (safe != null) {
          val bytes = JSONObject().put("binding",bound).put("report",safe).put("storedAt",System.currentTimeMillis()).toString().toByteArray()
          if (bytes.size <= 32768) atomic(File(directory,"event-${System.currentTimeMillis()}-${safe.getString("event_id")}.json"),bytes)
          trim()
        }
      }
    } }
    Function("collectPending") { synchronized(mutex) {
      val bound = binding()
      if (bound == null) { disarm(); emptyList<String>() } else {
        trim()
        records().mapNotNull { file -> runCatching {
          val wrapper = JSONObject(file.readText()); val saved = wrapper.getJSONObject("binding")
          if (listOf("accountId","revision","epochId").any { saved.getString(it) != bound.getString(it) }) { file.delete();null }
          else JSONObject().put("binding",saved).put("report",wrapper.getJSONObject("report")).toString()
        }.getOrNull() }
      }
    } }
    Function("removeRecords") { ids: List<String> -> synchronized(mutex) { records().forEach { file -> if (ids.any { runCatching { UUID.fromString(it) }.isSuccess && file.name.endsWith("-$it.json") }) file.delete() } } }
  }
}
