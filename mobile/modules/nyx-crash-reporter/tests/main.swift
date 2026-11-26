import Foundation

// Regression: real RCTFatal/SIGSEGV reports were ~375 KB, above the old raw cap.
precondition(crashRawReportMaxBytes == 4 * 1024 * 1024)
precondition(crashRawReportMaxBytes >= 2 * 1024 * 1024)
precondition(crashRawReportMaxBytes > 375_464)
precondition(crashRawReportMaxBytes > 373_327)
precondition(rawCrashReportDecision(byteCount: crashRawReportMaxBytes - 1) == .parse)
precondition(rawCrashReportDecision(byteCount: crashRawReportMaxBytes) == .parse)
precondition(rawCrashReportDecision(byteCount: crashRawReportMaxBytes + 1) == .discard(.oversize))
precondition(rawCrashReportDecision(byteCount: 1) == .parse)
precondition(rawCrashReportDecision(byteCount: 0) == .discard(.unparseable))
precondition(rawCrashReportDecision(byteCount: -1) == .discard(.unparseable))
precondition(
  nativeCrashThreadIndex([
    (crashed: false, frameAddresses: [0x10]),
    (crashed: true, frameAddresses: [0x20]),
  ], faultAddress: 0x10) == 1)
precondition(
  nativeCrashThreadIndex([
    (crashed: false, frameAddresses: [0x10]),
    (crashed: false, frameAddresses: [0x20, 0x30]),
  ], faultAddress: 0x30) == 1)
precondition(
  nativeCrashThreadIndex([
    (crashed: false, frameAddresses: [0x10]),
    (crashed: false, frameAddresses: [0x20]),
  ], faultAddress: 0x40) == 0)
precondition(nativeCrashThreadIndex([(crashed: false, frameAddresses: [])], faultAddress: 0) == 0)
precondition(nativeCrashThreadIndex([], faultAddress: 0) == nil)

let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: directory) }
for name in ["binding.json", "raw-authority.json", "event-1.json", "binding.json.tmp", "event-2.json.tmp", ".staging"] {
  try Data("sanitized private buffer".utf8).write(to: directory.appendingPathComponent(name))
}
try FileManager.default.createDirectory(
  at: directory.appendingPathComponent("raw"), withIntermediateDirectories: true)
purgeInterruptedCrashWrites(in: directory)
let retainedFiles = try FileManager.default.contentsOfDirectory(atPath: directory.path)
precondition(Set(retainedFiles) == Set(["binding.json", "raw-authority.json", "event-1.json", "raw"]))

precondition(!crashCaptureCanArm(reporterAvailable: false, enabled: false))
precondition(!crashCaptureCanArm(reporterAvailable: false, enabled: true))
precondition(!crashCaptureCanArm(reporterAvailable: true, enabled: false))
precondition(crashCaptureCanArm(reporterAvailable: true, enabled: true))
let epoch = UUID().uuidString
precondition(pendingCrashDecision(bindingEpoch: epoch, reportEpoch: epoch, jsEventId: nil) == .native)
precondition(pendingCrashDecision(bindingEpoch: nil, reportEpoch: epoch, jsEventId: nil) == .discard(.unauthorized))
precondition(
  pendingCrashDecision(bindingEpoch: epoch, reportEpoch: UUID().uuidString, jsEventId: nil)
    == .discard(.epochMismatch))
precondition(
  pendingCrashDecision(bindingEpoch: epoch, reportEpoch: epoch, jsEventId: UUID().uuidString)
    == .discard(.jsDuplicate))
precondition(pendingCrashDecision(bindingEpoch: "", reportEpoch: "", jsEventId: nil) == .discard(.unauthorized))
let secret = "SECRET_SENTINEL Bearer private person@example.test /Users/private eyJhbGci.jwt"
let containerId = "11111111-2222-4333-8444-555555555555"
let pathSentinel = "PATH_SENTINEL_Bearer_private_person@example.test"
let registerSentinel: UInt64 = 0x1234_ABCD_5678_0000
let nativeKeys: Set<String> = [
  "schema_version", "event_id", "captured_at", "platform", "kind", "exception_class",
  "native_signal", "app_version", "app_build", "os_version", "js_frames", "native_images",
  "native_frames",
]
let regressionNow = Date(timeIntervalSince1970: 1_800_000_000)
let regressionEventId = "22222222-3333-4444-8555-666666666666"
let fixtureImages: [ParsedNativeCrash.Image] = (0..<600).map { index in
  let uuid = String(format: "AABBCCDD-EEFF-4A55-8B66-%012X", index)
  let path: String
  if index == 0 {
    path = "/private/var/containers/Bundle/Application/\(containerId)/\(pathSentinel)/NyxIDMobile.app/NyxIDMobile"
  } else {
    let runtime = "/Library/Developer/CoreSimulator/Volumes/iOS_24C5089c/Library/Developer/CoreSimulator/Profiles/Runtimes/iOS 27.2.simruntime/Contents/Resources/RuntimeRoot"
    let framework = "/System/Library/PrivateFrameworks/AppleMediaServicesUI.framework/Frameworks/SimulatorMediaService\(index).framework/\(pathSentinel)"
    path = runtime + framework + "/usr/lib/system/libsystem_kernel.dylib"
  }
  return ParsedNativeCrash.Image(
    uuid: index % 2 == 0 ? uuid : uuid.replacingOccurrences(of: "-", with: ""),
    name: path, baseAddress: 0x1_0000_0000 + UInt64(index) * 0x20_0000, size: 0x10_0000)
}
let fixtureThreads: [ParsedNativeCrash.Thread] = (0..<40).map { thread in
  let frames: [UInt64] = (0..<64).map { frame in
    let base = fixtureImages[frame % 20].baseAddress
    let offset: UInt64
    if thread == 17 { offset = 0x2A0 + UInt64(frame) * 4 }
    else { offset = 0x1000 + UInt64(thread * 256 + frame * 4) }
    return base + offset
  }
  return ParsedNativeCrash.Thread(crashed: false, frames: frames)
}
let fixture = ParsedNativeCrash(
  threads: fixtureThreads, images: fixtureImages, signalName: "SIGSEGV",
  faultAddress: fixtureThreads[17].frames[0], appMarketingVersion: "1.0.4", appBuild: "45",
  osVersion: "27.2.0", timestamp: regressionNow.addingTimeInterval(-60))
// Model raw-file geometry, including register metadata that has no ParsedNativeCrash field.
// Use no padding: 40 x 64 instruction pointers, 34 registers/thread, 600 full image paths.
let rawFixture: [String: Any] = [
  "threads": fixtureThreads.map { thread -> [String: Any] in
    [
      "crashed": thread.crashed,
      "frames": thread.frames.map { ["instructionPointer": $0] },
      "registers": (0..<34).map { ["name": "x\($0)", "value": registerSentinel + UInt64($0)] as [String: Any] },
    ]
  },
  "images": fixtureImages.map {
    ["uuid": $0.uuid ?? "", "name": $0.name, "baseAddress": $0.baseAddress, "size": $0.size] as [String: Any]
  },
  "signalName": fixture.signalName ?? "", "faultAddress": fixture.faultAddress,
  "appMarketingVersion": fixture.appMarketingVersion ?? "", "appBuild": fixture.appBuild ?? "",
  "osVersion": fixture.osVersion, "timestamp": "2027-01-15T07:59:00Z", "customData": secret,
]
let rawFixtureData = try JSONSerialization.data(
  withJSONObject: rawFixture, options: [.sortedKeys, .withoutEscapingSlashes])
precondition(fixtureThreads.count == 40 && fixtureThreads.allSatisfy { !$0.crashed && $0.frames.count == 64 })
precondition(fixtureImages.count == 600)
precondition(rawFixtureData.count >= 375_000)
precondition(rawCrashReportDecision(byteCount: rawFixtureData.count) == .parse)
let mapped = sanitizedNativeCrash(fixture, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow)!
precondition(Set(mapped.keys) == nativeKeys)
precondition(mapped["event_id"] as? String == regressionEventId)
precondition(mapped["native_signal"] as? String == "SIGSEGV")
precondition(mapped["app_version"] as? String == "1.0.4")
precondition(mapped["app_build"] as? String == "45")
precondition(mapped["os_version"] as? String == "27.2")
let mappedFrames = mapped["native_frames"] as! [[String: Any]]
let mappedImages = mapped["native_images"] as! [[String: Any]]
precondition(mappedFrames.count == 64 && mappedImages.count == 16)
for (index, image) in mappedImages.enumerated() {
  precondition(Set(image.keys) == Set(["uuid", "name", "size"]))
  let uuid = image["uuid"] as! String
  precondition(uuid == uuid.lowercased())
  precondition(UUID(uuidString: uuid) != nil)
  precondition(uuid == String(format: "aabbccdd-eeff-4a55-8b66-%012x", index))
  let name = image["name"] as! String
  precondition(!name.contains("/") && name == URL(fileURLWithPath: fixtureImages[index].name).lastPathComponent)
  precondition((image["size"] as! UInt64) == fixtureImages[index].size)
}
for (index, frame) in mappedFrames.enumerated() {
  precondition(Set(frame.keys) == Set(["image", "offset"]))
  let offset = (frame["offset"] as! UInt64)
  if let image = frame["image"] as? Int {
    precondition(image == index % 20 && image < 16)
    precondition(offset == 0x2A0 + UInt64(index) * 4 && offset < fixtureImages[image].size)
  } else {
    precondition(frame["image"] is NSNull && index % 20 >= 16 && offset == 0)
  }
}
let mappedBytes = String(data: try JSONSerialization.data(withJSONObject: mapped), encoding: .utf8)!
let privateSentinels = [
  secret, "SECRET_SENTINEL", pathSentinel, "Bearer", "person@", "eyJhbGci", containerId,
  "/Library/", "/private/", "/Users/", "registers", "customData",
] + (0..<34).flatMap { index in
  let value = registerSentinel + UInt64(index)
  return [String(value), String(value, radix: 16)]
}
for value in privateSentinels { precondition(!mappedBytes.contains(value)) }
for thread in fixtureThreads {
  for address in thread.frames {
    precondition(!mappedBytes.contains(String(address)))
    precondition(!mappedBytes.contains(String(address, radix: 16)))
  }
}
for image in fixtureImages {
  precondition(!mappedBytes.contains(image.name))
  precondition(!mappedBytes.contains(String(image.baseAddress)))
}
// Exercise mapping limits and invalid parser fields at the same portable boundary.
var variant = fixture
variant.threads[17].frames.append(fixtureImages[0].baseAddress + 123)
precondition((sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow)!["native_frames"] as! [Any]).count == 64)
variant = fixture
variant.threads = []
precondition(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow) == nil)
variant = fixture
variant.faultAddress = 0
let firstThread = sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow)!
precondition(((firstThread["native_frames"] as! [[String: Any]])[0]["offset"] as! UInt64) == 0x1000)
variant.threads[5].crashed = true
let markedThread = sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow)!
precondition(((markedThread["native_frames"] as! [[String: Any]])[0]["offset"] as! UInt64) == 0x1500)
for invalidImage in [
  ParsedNativeCrash.Image(uuid: nil, name: "NyxIDMobile", baseAddress: fixtureImages[0].baseAddress, size: 0x10_0000),
  ParsedNativeCrash.Image(uuid: fixtureImages[0].uuid, name: "/private/\(secret)", baseAddress: fixtureImages[0].baseAddress, size: 0x10_0000),
] {
  variant = fixture
  variant.images[0] = invalidImage
  let record = sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow)!
  let frame = (record["native_frames"] as! [[String: Any]])[0]
  precondition(frame["image"] is NSNull && (frame["offset"] as! UInt64) == 0)
}
for signal in ["SIGSEGV", "SIGBUS", "SIGABRT", "SIGILL", "SIGFPE", "SIGTRAP", "SIGSYS", secret] {
  variant = fixture
  variant.signalName = signal
  let record = sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow)!
  precondition(record["native_signal"] as? String == (signal == secret ? "Other" : signal))
}
variant = fixture
variant.appMarketingVersion = secret
precondition(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow) == nil)
variant = fixture
variant.appBuild = secret
precondition(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow) == nil)
variant = fixture
variant.osVersion = secret
precondition(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow) == nil)
precondition(sanitizedNativeCrash(fixture, eventId: secret, now: regressionNow, fallbackCapturedAt: regressionNow) == nil)
variant = fixture
variant.timestamp = regressionNow.addingTimeInterval(-86401)
precondition(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow) == nil)
variant.timestamp = regressionNow.addingTimeInterval(301)
precondition(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow) == nil)

let logPattern = "^nyx-crash: raw report discarded reason=(oversize|unparseable|unauthorized|epoch_mismatch|expired|js_duplicate|reporter_unavailable) bytes=([0-9]+|unknown)$"
let logByteCounts: [Int?] = [nil, 0, rawFixtureData.count, crashRawReportMaxBytes + 1, Int.max, -1]
for reason in CrashDiscardReason.allCases {
  for count in logByteCounts {
    let line = crashDiscardLogLine(reason: reason, byteCount: count)
    precondition(crashMatches(line, logPattern))
    let expectedBytes = count.flatMap { $0 >= 0 ? String($0) : nil } ?? "unknown"
    precondition(line == "nyx-crash: raw report discarded reason=\(reason.rawValue) bytes=\(expectedBytes)")
    for value in privateSentinels { precondition(!line.contains(value)) }
  }
}
precondition(crashDiscardLogLine(reason: .oversize, byteCount: crashRawReportMaxBytes + 1)
  == "nyx-crash: raw report discarded reason=oversize bytes=4194305")
precondition(crashDiscardLogLine(reason: .unparseable, byteCount: nil)
  == "nyx-crash: raw report discarded reason=unparseable bytes=unknown")
precondition(crashDiscardLogLine(reason: .epochMismatch, byteCount: 123)
  == "nyx-crash: raw report discarded reason=epoch_mismatch bytes=123")
precondition(crashDiscardLogLine(reason: .jsDuplicate, byteCount: nil)
  == "nyx-crash: raw report discarded reason=js_duplicate bytes=unknown")
let noticePattern = "^nyx-crash: raw report age unknown bytes=([0-9]+|unknown)$"
for count in logByteCounts {
  let line = crashRawNoticeLogLine(reason: .ageUnknown, byteCount: count)
  let expectedBytes = count.flatMap { $0 >= 0 ? String($0) : nil } ?? "unknown"
  precondition(line == "nyx-crash: raw report age unknown bytes=\(expectedBytes)")
  precondition(crashMatches(line, noticePattern))
  precondition(!line.contains("discarded"))
  for value in privateSentinels { precondition(!line.contains(value)) }
}
precondition(crashRawNoticeLogLine(reason: .ageUnknown, byteCount: nil)
  == "nyx-crash: raw report age unknown bytes=unknown")
precondition(crashRawNoticeLogLine(reason: .ageUnknown, byteCount: 415006)
  == "nyx-crash: raw report age unknown bytes=415006")
let formatter = ISO8601DateFormatter()
formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
let base: [String: Any] = [
  "schema_version": 1, "event_id": UUID().uuidString, "captured_at": formatter.string(from: Date()),
  "platform": "ios", "kind": "js_fatal", "exception_class": "Error", "app_version": "1.2.3",
  "app_build": "123", "os_version": "18.1",
  "js_frames": [["asset": "main.jsbundle", "line": 1, "column": 2]], "native_images": [],
  "native_frames": [],
]
precondition(sanitizedFatalJs(base) != nil)
for key in ["reason", "registers", "message", "customData", "absolutePath", "extra"] {
  var bad = base
  bad[key] = secret
  precondition(sanitizedFatalJs(bad) == nil)
}
var badPath = base
badPath["js_frames"] = [["asset": "/Users/private/main.jsbundle", "line": 1, "column": 2]]
precondition(sanitizedFatalJs(badPath) == nil)
var native = base
native["kind"] = "native_crash"
native["exception_class"] = "Unknown"
native["native_signal"] = "SIGABRT"
native["js_frames"] = []
native["native_images"] = [
  ["uuid": UUID().uuidString, "name": "NyxIDMobile", "size": UInt64(4096), "registers": secret]
]
native["native_frames"] = [["image": 0, "offset": UInt64(32), "function": secret]]
for key in ["reason", "registers", "message", "customData", "absolutePath"] { native[key] = secret }
let output = sanitizedNativeRecord(native)!
precondition(Set(output.keys) == nativeKeys)
let bytes = String(data: try JSONSerialization.data(withJSONObject: output), encoding: .utf8)!
for value in [
  "SECRET_SENTINEL", "Bearer", "person@", "/Users", "eyJ", "reason", "registers", "message",
  "customData", "function",
] { precondition(!bytes.contains(value)) }
print("Generated native crash regression fixture: \(rawFixtureData.count) bytes (40 threads x 64 frames, 600 images)")
print("Native crash size decision, mapping, discard log, JS fatal rejection and native allowlist proofs passed")

for age: TimeInterval? in [0, 86399, 86400, -300, nil] {
  precondition(initialRawReportDecision(bindingEpoch: epoch, authorityEpoch: epoch, fileAgeSeconds: age) == .parse)
}
for age: TimeInterval? in [86400.01, -300.01, .infinity, .nan] {
  precondition(initialRawReportDecision(bindingEpoch: epoch, authorityEpoch: epoch, fileAgeSeconds: age) == .discard(.expired))
}
for pair: (String?, String?) in [(nil, epoch), (epoch, nil), ("", "")] {
  precondition(initialRawReportDecision(bindingEpoch: pair.0, authorityEpoch: pair.1, fileAgeSeconds: 1) == .discard(.unauthorized))
}
precondition(initialRawReportDecision(bindingEpoch: epoch, authorityEpoch: UUID().uuidString, fileAgeSeconds: 1) == .discard(.epochMismatch))
let parsedTimestampNow = Date(timeIntervalSince1970: 1_800_000_000)
precondition(parsedCrashTimestampDecision(timestamp: nil, now: parsedTimestampNow) == .continueParsing)
precondition(parsedCrashTimestampDecision(timestamp: parsedTimestampNow.addingTimeInterval(-86401), now: parsedTimestampNow) == .discard(.expired))
precondition(parsedCrashTimestampDecision(timestamp: parsedTimestampNow.addingTimeInterval(301), now: parsedTimestampNow) == .discard(.expired))

let capturedFormatter = ISO8601DateFormatter()
capturedFormatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
variant = fixture
variant.timestamp = nil
let mtime = regressionNow.addingTimeInterval(-120)
let mtimeRecord = sanitizedNativeCrash(
  variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: mtime)!
precondition(mtimeRecord["captured_at"] as? String == capturedFormatter.string(from: mtime))
let collectionRecord = sanitizedNativeCrash(
  variant, eventId: regressionEventId, now: regressionNow, fallbackCapturedAt: regressionNow)!
precondition(collectionRecord["captured_at"] as? String == capturedFormatter.string(from: regressionNow))
variant.timestamp = regressionNow.addingTimeInterval(-60)
let presentRecord = sanitizedNativeCrash(
  variant, eventId: regressionEventId, now: regressionNow,
  fallbackCapturedAt: regressionNow.addingTimeInterval(200))!
precondition(presentRecord["captured_at"] as? String == capturedFormatter.string(from: variant.timestamp!))
variant.timestamp = nil
precondition(sanitizedNativeCrash(
  variant, eventId: regressionEventId, now: regressionNow,
  fallbackCapturedAt: regressionNow.addingTimeInterval(-86401)) == nil)
precondition(sanitizedNativeCrash(
  variant, eventId: regressionEventId, now: regressionNow,
  fallbackCapturedAt: regressionNow.addingTimeInterval(301)) == nil)

// The module and OnCreate share this source-independent consume path. The fake
// records calls so unread purges and persist-before-purge ordering stay tested
// on Linux as well as macOS, without importing PLCrashReporter.
final class FakePendingRawSource: PendingRawSource {
  let hasPending: Bool
  let modificationDate: Date?
  let data: Data?
  var calls: [String] = []

  init(hasPending: Bool = true, modificationDate: Date?, data: Data? = Data("raw".utf8)) {
    self.hasPending = hasPending
    self.modificationDate = modificationDate
    self.data = data
  }

  func loadData() -> Data? {
    calls.append("load")
    return data
  }

  func purge() {
    calls.append("purge")
  }
}

func pending(_ crash: ParsedNativeCrash, reportEpoch: String? = epoch, jsEventId: String? = nil)
  -> ParsedPendingRaw
{
  ParsedPendingRaw(crash: crash, reportEpoch: reportEpoch, jsEventId: jsEventId)
}

let consumeData = Data("authorized pending report".utf8)
let freshMtime = regressionNow.addingTimeInterval(-120)
let freshSource = FakePendingRawSource(modificationDate: freshMtime, data: consumeData)
var freshPersisted: [String: Any]?
var freshLogs: [String] = []
let freshRecord = consumePendingRaw(
  source: freshSource,
  bindingEpoch: epoch,
  authorityEpoch: epoch,
  now: regressionNow,
  parse: { data in
    precondition(data == consumeData)
    return pending(fixture)
  },
  persist: { record in
    precondition(freshSource.calls == ["load"])
    freshPersisted = record
  },
  emit: { freshLogs.append($0) })
precondition(freshRecord != nil && freshPersisted != nil)
precondition(freshSource.calls == ["load", "purge"])
precondition(freshLogs.isEmpty)

var nilMtimeCrash = fixture
nilMtimeCrash.timestamp = nil
let nilMtimeSource = FakePendingRawSource(modificationDate: nil, data: consumeData)
var nilMtimeLogs: [String] = []
let nilMtimeRecord = consumePendingRaw(
  source: nilMtimeSource,
  bindingEpoch: epoch,
  authorityEpoch: epoch,
  now: regressionNow,
  parse: { _ in pending(nilMtimeCrash) },
  persist: { _ in },
  emit: { nilMtimeLogs.append($0) })
precondition(nilMtimeRecord != nil)
precondition(nilMtimeSource.calls == ["load", "purge"])
precondition(nilMtimeLogs == ["nyx-crash: raw report age unknown bytes=\(consumeData.count)"])
precondition((nilMtimeRecord?["captured_at"] as? String) == capturedFormatter.string(from: regressionNow))

func assertUnreadPurge(
  bindingEpoch: String?, authorityEpoch: String?, modificationDate: Date?, logReason: String
) {
  let source = FakePendingRawSource(modificationDate: modificationDate, data: consumeData)
  var parseCalled = false
  var logs: [String] = []
  let record = consumePendingRaw(
    source: source,
    bindingEpoch: bindingEpoch,
    authorityEpoch: authorityEpoch,
    now: regressionNow,
    parse: { _ in
      parseCalled = true
      return pending(fixture)
    },
    persist: { _ in },
    emit: { logs.append($0) })
  precondition(record == nil)
  precondition(!parseCalled)
  precondition(source.calls == ["purge"])
  precondition(logs == ["nyx-crash: raw report discarded reason=\(logReason) bytes=unknown"])
}

assertUnreadPurge(bindingEpoch: nil, authorityEpoch: epoch, modificationDate: freshMtime, logReason: "unauthorized")
assertUnreadPurge(bindingEpoch: epoch, authorityEpoch: nil, modificationDate: freshMtime, logReason: "unauthorized")
assertUnreadPurge(
  bindingEpoch: epoch, authorityEpoch: UUID().uuidString, modificationDate: freshMtime,
  logReason: "epoch_mismatch")
assertUnreadPurge(
  bindingEpoch: epoch, authorityEpoch: epoch,
  modificationDate: regressionNow.addingTimeInterval(-crashRecordMaxAgeSeconds - 1),
  logReason: "expired")
let noPendingSource = FakePendingRawSource(hasPending: false, modificationDate: freshMtime)
var noPendingParseCalled = false
precondition(
  consumePendingRaw(
    source: noPendingSource,
    bindingEpoch: epoch,
    authorityEpoch: epoch,
    now: regressionNow,
    parse: { _ in
      noPendingParseCalled = true
      return pending(fixture)
    },
    persist: { _ in }) == nil)
precondition(!noPendingParseCalled && noPendingSource.calls == ["purge"])

let oversizeSource = FakePendingRawSource(
  modificationDate: freshMtime,
  data: Data(repeating: 0, count: crashRawReportMaxBytes + 1))
var oversizeParseCalled = false
var oversizeLogs: [String] = []
precondition(
  consumePendingRaw(
    source: oversizeSource,
    bindingEpoch: epoch,
    authorityEpoch: epoch,
    now: regressionNow,
    parse: { _ in
      oversizeParseCalled = true
      return pending(fixture)
    },
    persist: { _ in },
    emit: { oversizeLogs.append($0) }) == nil)
precondition(!oversizeParseCalled && oversizeSource.calls == ["load", "purge"])
precondition(oversizeLogs == ["nyx-crash: raw report discarded reason=oversize bytes=\(crashRawReportMaxBytes + 1)"])

func assertParsedDiscard(_ value: ParsedPendingRaw?, reason: String) {
  let source = FakePendingRawSource(modificationDate: freshMtime, data: consumeData)
  var parseCalled = false
  var logs: [String] = []
  precondition(
    consumePendingRaw(
      source: source,
      bindingEpoch: epoch,
      authorityEpoch: epoch,
      now: regressionNow,
      parse: { _ in
        parseCalled = true
        return value
      },
      persist: { _ in },
      emit: { logs.append($0) }) == nil)
  precondition(parseCalled && source.calls == ["load", "purge"])
  precondition(logs == ["nyx-crash: raw report discarded reason=\(reason) bytes=\(consumeData.count)"])
}

assertParsedDiscard(nil, reason: "unparseable")
assertParsedDiscard(pending(fixture, jsEventId: regressionEventId), reason: "js_duplicate")
assertParsedDiscard(pending(fixture, reportEpoch: UUID().uuidString), reason: "epoch_mismatch")
var expiredParsedCrash = fixture
expiredParsedCrash.timestamp = regressionNow.addingTimeInterval(-crashRecordMaxAgeSeconds - 1)
assertParsedDiscard(pending(expiredParsedCrash), reason: "expired")

let noticeReasons: [CrashRawNotice] = [.ageUnknown]
for noticeReason in noticeReasons {
  let notice = crashRawNoticeLogLine(reason: noticeReason, byteCount: nil)
  precondition(notice == "nyx-crash: raw report age unknown bytes=unknown")
  precondition(crashMatches(notice, "^nyx-crash: raw report age unknown bytes=([0-9]+|unknown)$"))
}


let bufferNow = Date(timeIntervalSince1970: 1_800_000_000)
let bufferFormatter = ISO8601DateFormatter()
bufferFormatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
for age in [0.0, 86400.0, -300.0] {
  precondition(crashRecordIsFresh(
    capturedAt: bufferFormatter.string(from: bufferNow.addingTimeInterval(-age)), now: bufferNow))
}
for age in [86400.001, -300.001] {
  precondition(!crashRecordIsFresh(
    capturedAt: bufferFormatter.string(from: bufferNow.addingTimeInterval(-age)), now: bufferNow))
}
for invalid in [nil, "not a timestamp", 123, String(repeating: "1", count: 41)] as [Any?] {
  precondition(!crashRecordIsFresh(capturedAt: invalid, now: bufferNow))
}
print("Captured-at buffer expiry proof passed")

print("Shared pending-raw consume proof passed")
