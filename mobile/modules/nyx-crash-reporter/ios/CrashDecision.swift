import Foundation

// Real iOS simulator reports measured ~375 KB (375,464 bytes for RCTFatal).
// This raw parsing/capture cap is separate from the 256 KiB sanitized buffer.
let crashRawReportMaxBytes = 4 * 1024 * 1024

enum CrashDiscardReason: String, CaseIterable {
  case oversize
  case unparseable
  case unauthorized
  case epochMismatch = "epoch_mismatch"
  case expired
  case jsDuplicate = "js_duplicate"
  case reporterUnavailable = "reporter_unavailable"
}
enum RawCrashReportDecision: Equatable { case parse, discard(CrashDiscardReason) }

func rawCrashReportDecision(byteCount: Int) -> RawCrashReportDecision {
  guard byteCount > 0 else { return .discard(.unparseable) }
  return byteCount <= crashRawReportMaxBytes ? .parse : .discard(.oversize)
}

/// Only a closed reason and a nonnegative byte count can enter the metadata log.
func crashDiscardLogLine(reason: CrashDiscardReason, byteCount: Int?) -> String {
  let bytes = byteCount.flatMap { $0 >= 0 ? String($0) : nil } ?? "unknown"
  return "nyx-crash: raw report discarded reason=\(reason.rawValue) bytes=\(bytes)"
}

enum CrashRawNotice: String { case ageUnknown = "age_unknown" }

func crashRawNoticeLogLine(reason: CrashRawNotice, byteCount: Int?) -> String {
  let bytes = byteCount.flatMap { $0 >= 0 ? String($0) : nil } ?? "unknown"
  switch reason {
  case .ageUnknown:
    return "nyx-crash: raw report age unknown bytes=\(bytes)"
  }
}

let crashRecordMaxAgeSeconds: TimeInterval = 86400
/// Age follows capture time, never the time a crash was copied into the buffer.
func crashRecordIsFresh(capturedAt: Any?, now: Date) -> Bool {
  guard let value = capturedAt as? String, value.utf8.count <= 40 else { return false }
  let formatter = ISO8601DateFormatter()
  formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
  var date = formatter.date(from: value)
  if date == nil {
    formatter.formatOptions = [.withInternetDateTime]
    date = formatter.date(from: value)
  }
  guard let date else { return false }
  let age = now.timeIntervalSince(date)
  return age.isFinite && age >= -300 && age <= crashRecordMaxAgeSeconds
}

enum InitialRawReportDecision: Equatable { case parse, discard(CrashDiscardReason) }
/// An authorized report with an unavailable mtime remains parseable. The
/// parsed report customData is checked separately after the in-memory parse.
func initialRawReportDecision(bindingEpoch: String?, authorityEpoch: String?, fileAgeSeconds: TimeInterval?) -> InitialRawReportDecision {
  guard let epoch = bindingEpoch, !epoch.isEmpty else { return .discard(.unauthorized) }
  guard let authorityEpoch, !authorityEpoch.isEmpty else { return .discard(.unauthorized) }
  guard epoch == authorityEpoch else { return .discard(.epochMismatch) }
  guard let age = fileAgeSeconds else { return .parse }
  guard age.isFinite, age >= -300, age <= crashRecordMaxAgeSeconds else {
    return .discard(.expired)
  }
  return .parse
}

enum ParsedCrashTimestampDecision: Equatable { case continueParsing, discard(CrashDiscardReason) }

func parsedCrashTimestampDecision(timestamp: Date?, now: Date) -> ParsedCrashTimestampDecision {
  guard let timestamp else { return .continueParsing }
  let age = now.timeIntervalSince(timestamp)
  guard age.isFinite, age >= -300, age <= crashRecordMaxAgeSeconds else {
    return .discard(.expired)
  }
  return .continueParsing
}

protocol PendingRawSource {
  var hasPending: Bool { get }
  var modificationDate: Date? { get }
  func loadData() -> Data?
  func purge()
}

struct ParsedPendingRaw {
  var crash: ParsedNativeCrash
  var reportEpoch: String?
  var jsEventId: String?
}

private func emitPendingRawLog(_ line: String) {
  NSLog("%@", line)
}

func consumePendingRaw(
  source: PendingRawSource,
  bindingEpoch: String?,
  authorityEpoch: String?,
  now: Date,
  parse: (Data) -> ParsedPendingRaw?,
  persist: ([String: Any]) -> Void,
  emit: (String) -> Void = emitPendingRawLog
) -> [String: Any]? {
  guard source.hasPending else {
    source.purge()
    return nil
  }
  let modificationDate = source.modificationDate
  let fileAge = modificationDate.map { now.timeIntervalSince($0) }
  defer { source.purge() }

  switch initialRawReportDecision(
    bindingEpoch: bindingEpoch, authorityEpoch: authorityEpoch, fileAgeSeconds: fileAge)
  {
  case .discard(let reason):
    emit(crashDiscardLogLine(reason: reason, byteCount: nil))
    return nil
  case .parse:
    break
  }

  let data = source.loadData()
  let byteCount = data?.count
  if modificationDate == nil {
    emit(crashRawNoticeLogLine(reason: .ageUnknown, byteCount: byteCount))
  }
  switch rawCrashReportDecision(byteCount: byteCount ?? 0) {
  case .discard(let reason):
    emit(crashDiscardLogLine(reason: reason, byteCount: byteCount))
    return nil
  case .parse:
    break
  }
  guard let data, let parsed = parse(data) else {
    emit(crashDiscardLogLine(reason: .unparseable, byteCount: byteCount))
    return nil
  }
  switch pendingCrashDecision(
    bindingEpoch: bindingEpoch, reportEpoch: parsed.reportEpoch, jsEventId: parsed.jsEventId)
  {
  case .discard(let reason):
    emit(crashDiscardLogLine(reason: reason, byteCount: byteCount))
    return nil
  case .native:
    break
  }
  switch parsedCrashTimestampDecision(timestamp: parsed.crash.timestamp, now: now) {
  case .discard(let reason):
    emit(crashDiscardLogLine(reason: reason, byteCount: byteCount))
    return nil
  case .continueParsing:
    break
  }
  guard let record = sanitizedNativeCrash(
    parsed.crash,
    eventId: UUID().uuidString.lowercased(),
    now: now,
    fallbackCapturedAt: modificationDate ?? now)
  else {
    emit(crashDiscardLogLine(reason: .unparseable, byteCount: byteCount))
    return nil
  }
  persist(record)
  return record
}

struct NativeCrashImage { var uuid: String; var name: String; var size: UInt64 }
struct NativeCrashFrame { var image: Int?; var offset: UInt64 }

/// Parser-independent values only. Exception text, registers and other parser metadata have no field.
struct ParsedNativeCrash {
  struct Thread {
    var crashed: Bool
    var frames: [UInt64]
  }
  struct Image {
    var uuid: String?
    var name: String
    var baseAddress: UInt64
    var size: UInt64
  }
  var threads: [Thread]
  var images: [Image]
  var signalName: String?
  var faultAddress: UInt64
  var appMarketingVersion: String?
  var appBuild: String?
  var osVersion: String
  var timestamp: Date?
}

func sanitizedNativeCrash(
  _ crash: ParsedNativeCrash, eventId: String, now: Date, fallbackCapturedAt: Date
) -> [String: Any]? {
  let timestamp = crash.timestamp ?? fallbackCapturedAt
  guard let threadIndex = nativeCrashThreadIndex(
    crash.threads.map { (crashed: $0.crashed, frameAddresses: $0.frames) },
    faultAddress: crash.faultAddress),
    timestamp.timeIntervalSince(now) >= -86400,
    timestamp.timeIntervalSince(now) <= 300
  else { return nil }
  var selected: [ParsedNativeCrash.Image] = []
  var images: [NativeCrashImage] = []
  var frames: [NativeCrashFrame] = []
  for address in crash.threads[threadIndex].frames.prefix(64) {
    guard let image = crash.images.first(where: {
      address >= $0.baseAddress && address - $0.baseAddress < $0.size
    }), let rawUuid = image.uuid
    else {
      frames.append(NativeCrashFrame(image: nil, offset: 0))
      continue
    }
    let compact = rawUuid.replacingOccurrences(of: "-", with: "")
    guard compact.count == 32 else { continue }
    let uuidText =
      "\(compact.prefix(8))-\(compact.dropFirst(8).prefix(4))-\(compact.dropFirst(12).prefix(4))-\(compact.dropFirst(16).prefix(4))-\(compact.suffix(12))"
    guard UUID(uuidString: uuidText) != nil else { continue }
    let name = URL(fileURLWithPath: image.name).lastPathComponent
    guard crashMatches(name, "^[A-Za-z0-9_.+-]{1,64}$") else {
      frames.append(NativeCrashFrame(image: nil, offset: 0))
      continue
    }
    var index = selected.firstIndex(where: { $0.baseAddress == image.baseAddress })
    if index == nil && selected.count < 16 {
      index = selected.count
      selected.append(image)
      images.append(NativeCrashImage(uuid: uuidText.lowercased(), name: name, size: image.size))
    }
    if let index {
      frames.append(NativeCrashFrame(image: index, offset: address - image.baseAddress))
    } else {
      frames.append(NativeCrashFrame(image: nil, offset: 0))
    }
  }
  let version = crash.appMarketingVersion ?? "0"
  let build = crash.appBuild ?? "0"
  guard crashMatches(version, "^[0-9A-Za-z.+-]{1,32}$"),
    crashMatches(build, "^[0-9A-Za-z.+-]{1,64}$")
  else { return nil }
  let os = crash.osVersion.split(separator: ".").prefix(2).joined(separator: ".")
  guard crashMatches(os, "^[0-9]{1,3}(\\.[0-9]{1,3})?$") else { return nil }
  let signal = crash.signalName ?? "Other"
  let formatter = ISO8601DateFormatter()
  formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
  return sanitizedNativeRecord([
    "schema_version": 1, "event_id": eventId, "captured_at": formatter.string(from: timestamp),
    "platform": "ios", "kind": "native_crash", "exception_class": "Unknown",
    "native_signal": ["SIGSEGV", "SIGBUS", "SIGABRT", "SIGILL", "SIGFPE", "SIGTRAP", "SIGSYS"]
      .contains(signal) ? signal : "Other", "app_version": version, "app_build": build,
    "os_version": os, "js_frames": [], "native_images": images.map { ["uuid": $0.uuid, "name": $0.name, "size": $0.size] as [String: Any] }, "native_frames": frames.map { ["image": $0.image.map { $0 as Any } ?? NSNull(), "offset": $0.offset] as [String: Any] },
  ])
}

/// Prefer the crash marker; externally delivered signals may leave every thread unmarked.
func nativeCrashThreadIndex(
  _ threads: [(crashed: Bool, frameAddresses: [UInt64])], faultAddress: UInt64
) -> Int? {
  threads.firstIndex(where: { $0.crashed })
    ?? threads.firstIndex(where: { $0.frameAddresses.contains(faultAddress) })
    ?? threads.indices.first
}

func crashCaptureCanArm(reporterAvailable: Bool, enabled: Bool) -> Bool {
  reporterAvailable && enabled
}

/// A crash during the atomic write can leave staging files. They never become
/// records and must not outlive the next buffer operation or a withdrawal.
func purgeInterruptedCrashWrites(in directory: URL) {
  let files =
    (try? FileManager.default.contentsOfDirectory(
      at: directory, includingPropertiesForKeys: [.isRegularFileKey])) ?? []
  for file in files {
    let name = file.lastPathComponent
    guard name != "binding.json", name != "raw-authority.json",
      !(name.hasPrefix("event-") && file.pathExtension == "json"),
      (try? file.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
    else { continue }
    try? FileManager.default.removeItem(at: file)
  }
}

/// Pure policy: inspect only the arming token and JS fatal marker, never exception text/registers.
enum PendingCrashDecision: Equatable { case discard(CrashDiscardReason), native }
func pendingCrashDecision(bindingEpoch: String?, reportEpoch: String?, jsEventId: String?)
  -> PendingCrashDecision
{
  guard let bindingEpoch, !bindingEpoch.isEmpty else { return .discard(.unauthorized) }
  guard let reportEpoch, !reportEpoch.isEmpty else { return .discard(.unauthorized) }
  guard bindingEpoch == reportEpoch else { return .discard(.epochMismatch) }
  guard jsEventId == nil else { return .discard(.jsDuplicate) }
  return .native
}
func crashMatches(_ value: String, _ pattern: String) -> Bool {
  value.range(of: pattern, options: .regularExpression) != nil
}
/// Second native boundary for fatal JS. Reject unknown keys, then reconstruct a closed record.
func sanitizedFatalJs(_ value: Any) -> [String: Any]? {
  guard let v = value as? [String: Any],
    Set(v.keys).isSubset(
      of: Set([
        "schema_version", "event_id", "captured_at", "platform", "kind", "exception_class",
        "app_version", "app_build", "artifact_id", "os_version", "runtime", "js_frames",
        "native_images", "native_frames",
      ])),
    v["schema_version"] as? Int == 1, v["kind"] as? String == "js_fatal",
    v["platform"] as? String == "ios",
    let event = v["event_id"] as? String, UUID(uuidString: event) != nil,
    let captured = v["captured_at"] as? String, captured.count <= 40,
    let version = v["app_version"] as? String, crashMatches(version, "^[0-9A-Za-z.+-]{1,32}$"),
    let build = v["app_build"] as? String, crashMatches(build, "^[0-9A-Za-z.+-]{1,64}$"),
    let os = v["os_version"] as? String, crashMatches(os, "^[0-9]{1,3}(\\.[0-9]{1,3})?$"),
    let cls = v["exception_class"] as? String,
    [
      "Error", "TypeError", "ReferenceError", "RangeError", "SyntaxError", "EvalError", "URIError",
      "AggregateError", "ChunkLoadError", "NonError", "Other", "Unknown",
    ].contains(cls),
    let frames = v["js_frames"] as? [[String: Any]], frames.count <= 64,
    let images = v["native_images"] as? [Any], images.isEmpty,
    let native = v["native_frames"] as? [Any], native.isEmpty
  else { return nil }
  let formatter = ISO8601DateFormatter()
  formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
  guard let date = formatter.date(from: captured), date.timeIntervalSinceNow >= -86400,
    date.timeIntervalSinceNow <= 300
  else { return nil }
  var safeFrames: [[String: Any]] = []
  for frame in frames {
    guard Set(frame.keys) == Set(["asset", "line", "column"]),
      let asset = frame["asset"] as? String,
      ["main.jsbundle", "index.android.bundle", "<native>", "<external>"].contains(asset),
      let line = frame["line"] as? Int, let column = frame["column"] as? Int,
      (0...10_000_000).contains(line), (0...10_000_000).contains(column)
    else { return nil }
    safeFrames.append(["asset": asset, "line": line, "column": column])
  }
  var result: [String: Any] = [
    "schema_version": 1, "event_id": event, "captured_at": captured,
    "platform": "ios", "kind": "js_fatal", "exception_class": cls,
    "app_version": version, "app_build": build, "os_version": os, "js_frames": safeFrames,
    "native_images": [], "native_frames": [],
  ]
  if let artifact = v["artifact_id"] as? String {
    guard UUID(uuidString: artifact) != nil else { return nil }
    result["artifact_id"] = artifact
  }
  if let runtime = v["runtime"] as? [String: Any] {
    guard Set(runtime.keys) == Set(["name", "major"]), runtime["name"] as? String == "hermes",
      let major = runtime["major"] as? Int, (0...65535).contains(major)
    else { return nil }
    result["runtime"] = ["name": "hermes", "major": major]
  } else if v["runtime"] != nil {
    return nil
  }
  return result
}

/// Closed native output boundary. Extra parser metadata never enters the record.
func sanitizedNativeRecord(_ input: [String: Any]) -> [String: Any]? {
  let required = [
    "schema_version", "event_id", "captured_at", "platform", "kind", "exception_class",
    "native_signal", "app_version", "app_build", "os_version", "js_frames", "native_images",
    "native_frames",
  ]
  guard required.allSatisfy({ input[$0] != nil }), input["schema_version"] as? Int == 1,
    input["platform"] as? String == "ios", input["kind"] as? String == "native_crash",
    input["exception_class"] as? String == "Unknown",
    let event = input["event_id"] as? String, UUID(uuidString: event) != nil,
    let captured = input["captured_at"] as? String, captured.count <= 40,
    let version = input["app_version"] as? String, crashMatches(version, "^[0-9A-Za-z.+-]{1,32}$"),
    let build = input["app_build"] as? String, crashMatches(build, "^[0-9A-Za-z.+-]{1,64}$"),
    let os = input["os_version"] as? String, crashMatches(os, "^[0-9]{1,3}(\\.[0-9]{1,3})?$"),
    let signal = input["native_signal"] as? String,
    ["SIGSEGV", "SIGBUS", "SIGABRT", "SIGILL", "SIGFPE", "SIGTRAP", "SIGSYS", "Other"].contains(
      signal),
    let js = input["js_frames"] as? [Any], js.isEmpty,
    let images = input["native_images"] as? [[String: Any]], images.count <= 16,
    let frames = input["native_frames"] as? [[String: Any]], frames.count <= 64
  else { return nil }
  var safeImages: [[String: Any]] = []
  var safeFrames: [[String: Any]] = []
  for image in images {
    guard let uuid = image["uuid"] as? String, UUID(uuidString: uuid) != nil,
      let name = image["name"] as? String, crashMatches(name, "^[A-Za-z0-9_.+-]{1,64}$"),
      let size = image["size"] as? UInt64, size > 0,
      size <= UInt64(Int64.max)
    else { return nil }
    safeImages.append(["uuid": uuid, "name": name, "size": size])
  }
  for frame in frames {
    guard let offset = frame["offset"] as? UInt64, offset <= UInt64(Int64.max) else {
      return nil
    }
    if let index = frame["image"] as? Int {
      guard images.indices.contains(index), let size = images[index]["size"] as? UInt64,
        offset < size
      else { return nil }
      safeFrames.append(["image": index, "offset": offset])
    } else {
      safeFrames.append(["image": NSNull(), "offset": UInt64(0)])
    }
  }
  return [
    "schema_version": 1, "event_id": event, "captured_at": captured, "platform": "ios",
    "kind": "native_crash", "exception_class": "Unknown", "native_signal": signal,
    "app_version": version, "app_build": build, "os_version": os, "js_frames": [],
    "native_images": safeImages, "native_frames": safeFrames,
  ]
}
