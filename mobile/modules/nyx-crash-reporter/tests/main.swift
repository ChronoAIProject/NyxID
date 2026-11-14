import Foundation

// Regression: real RCTFatal/SIGSEGV reports were ~375 KB, above the old raw cap.
assert(crashRawReportMaxBytes == 4 * 1024 * 1024)
assert(crashRawReportMaxBytes >= 2 * 1024 * 1024)
assert(crashRawReportMaxBytes > 375_464)
assert(crashRawReportMaxBytes > 373_327)
assert(rawCrashReportDecision(byteCount: crashRawReportMaxBytes - 1) == .parse)
assert(rawCrashReportDecision(byteCount: crashRawReportMaxBytes) == .parse)
assert(rawCrashReportDecision(byteCount: crashRawReportMaxBytes + 1) == .discard(.oversize))
assert(rawCrashReportDecision(byteCount: 1) == .parse)
assert(rawCrashReportDecision(byteCount: 0) == .discard(.unparseable))
assert(rawCrashReportDecision(byteCount: -1) == .discard(.unparseable))
assert(
  nativeCrashThreadIndex([
    (crashed: false, frameAddresses: [0x10]),
    (crashed: true, frameAddresses: [0x20]),
  ], faultAddress: 0x10) == 1)
assert(
  nativeCrashThreadIndex([
    (crashed: false, frameAddresses: [0x10]),
    (crashed: false, frameAddresses: [0x20, 0x30]),
  ], faultAddress: 0x30) == 1)
assert(
  nativeCrashThreadIndex([
    (crashed: false, frameAddresses: [0x10]),
    (crashed: false, frameAddresses: [0x20]),
  ], faultAddress: 0x40) == 0)
assert(nativeCrashThreadIndex([(crashed: false, frameAddresses: [])], faultAddress: 0) == 0)
assert(nativeCrashThreadIndex([], faultAddress: 0) == nil)

let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: directory) }
for name in ["binding.json", "event-1.json", "binding.json.tmp", "event-2.json.tmp", ".staging"] {
  try Data("sanitized private buffer".utf8).write(to: directory.appendingPathComponent(name))
}
try FileManager.default.createDirectory(
  at: directory.appendingPathComponent("raw"), withIntermediateDirectories: true)
purgeInterruptedCrashWrites(in: directory)
let retainedFiles = try FileManager.default.contentsOfDirectory(atPath: directory.path)
assert(Set(retainedFiles) == Set(["binding.json", "event-1.json", "raw"]))

assert(!crashCaptureCanArm(reporterAvailable: false, enabled: false))
assert(!crashCaptureCanArm(reporterAvailable: false, enabled: true))
assert(!crashCaptureCanArm(reporterAvailable: true, enabled: false))
assert(crashCaptureCanArm(reporterAvailable: true, enabled: true))
let epoch = UUID().uuidString
assert(pendingCrashDecision(bindingEpoch: epoch, reportEpoch: epoch, jsEventId: nil) == .native)
assert(pendingCrashDecision(bindingEpoch: nil, reportEpoch: epoch, jsEventId: nil) == .discard)
assert(
  pendingCrashDecision(bindingEpoch: epoch, reportEpoch: UUID().uuidString, jsEventId: nil)
    == .discard)
assert(
  pendingCrashDecision(bindingEpoch: epoch, reportEpoch: epoch, jsEventId: UUID().uuidString)
    == .discard)
assert(pendingCrashDecision(bindingEpoch: "", reportEpoch: "", jsEventId: nil) == .discard)
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
assert(fixtureThreads.count == 40 && fixtureThreads.allSatisfy { !$0.crashed && $0.frames.count == 64 })
assert(fixtureImages.count == 600)
assert(rawFixtureData.count >= 375_000)
assert(rawCrashReportDecision(byteCount: rawFixtureData.count) == .parse)
let mapped = sanitizedNativeCrash(fixture, eventId: regressionEventId, now: regressionNow)!
assert(Set(mapped.keys) == nativeKeys)
assert(mapped["event_id"] as? String == regressionEventId)
assert(mapped["native_signal"] as? String == "SIGSEGV")
assert(mapped["app_version"] as? String == "1.0.4")
assert(mapped["app_build"] as? String == "45")
assert(mapped["os_version"] as? String == "27.2")
let mappedFrames = mapped["native_frames"] as! [[String: Any]]
let mappedImages = mapped["native_images"] as! [[String: Any]]
assert(mappedFrames.count == 64 && mappedImages.count == 16)
for (index, image) in mappedImages.enumerated() {
  assert(Set(image.keys) == Set(["uuid", "name", "size"]))
  let uuid = image["uuid"] as! String
  assert(uuid == uuid.lowercased())
  assert(UUID(uuidString: uuid) != nil)
  assert(uuid == String(format: "aabbccdd-eeff-4a55-8b66-%012x", index))
  let name = image["name"] as! String
  assert(!name.contains("/") && name == URL(fileURLWithPath: fixtureImages[index].name).lastPathComponent)
  assert((image["size"] as! NSNumber).uint64Value == fixtureImages[index].size)
}
for (index, frame) in mappedFrames.enumerated() {
  assert(Set(frame.keys) == Set(["image", "offset"]))
  let offset = (frame["offset"] as! NSNumber).uint64Value
  if let image = frame["image"] as? Int {
    assert(image == index % 20 && image < 16)
    assert(offset == 0x2A0 + UInt64(index) * 4 && offset < fixtureImages[image].size)
  } else {
    assert(frame["image"] is NSNull && index % 20 >= 16 && offset == 0)
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
for value in privateSentinels { assert(!mappedBytes.contains(value)) }
for thread in fixtureThreads {
  for address in thread.frames {
    assert(!mappedBytes.contains(String(address)))
    assert(!mappedBytes.contains(String(address, radix: 16)))
  }
}
for image in fixtureImages {
  assert(!mappedBytes.contains(image.name))
  assert(!mappedBytes.contains(String(image.baseAddress)))
}
// Exercise mapping limits and invalid parser fields at the same portable boundary.
var variant = fixture
variant.threads[17].frames.append(fixtureImages[0].baseAddress + 123)
assert((sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow)!["native_frames"] as! [Any]).count == 64)
variant = fixture
variant.threads = []
assert(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow) == nil)
variant = fixture
variant.faultAddress = 0
let firstThread = sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow)!
assert(((firstThread["native_frames"] as! [[String: Any]])[0]["offset"] as! NSNumber).uint64Value == 0x1000)
variant.threads[5].crashed = true
let markedThread = sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow)!
assert(((markedThread["native_frames"] as! [[String: Any]])[0]["offset"] as! NSNumber).uint64Value == 0x1500)
for invalidImage in [
  ParsedNativeCrash.Image(uuid: nil, name: "NyxIDMobile", baseAddress: fixtureImages[0].baseAddress, size: 0x10_0000),
  ParsedNativeCrash.Image(uuid: fixtureImages[0].uuid, name: "/private/\(secret)", baseAddress: fixtureImages[0].baseAddress, size: 0x10_0000),
] {
  variant = fixture
  variant.images[0] = invalidImage
  let record = sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow)!
  let frame = (record["native_frames"] as! [[String: Any]])[0]
  assert(frame["image"] is NSNull && (frame["offset"] as! NSNumber).uint64Value == 0)
}
for signal in ["SIGSEGV", "SIGBUS", "SIGABRT", "SIGILL", "SIGFPE", "SIGTRAP", "SIGSYS", secret] {
  variant = fixture
  variant.signalName = signal
  let record = sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow)!
  assert(record["native_signal"] as? String == (signal == secret ? "Other" : signal))
}
variant = fixture
variant.appMarketingVersion = secret
assert(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow) == nil)
variant = fixture
variant.appBuild = secret
assert(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow) == nil)
variant = fixture
variant.osVersion = secret
assert(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow) == nil)
assert(sanitizedNativeCrash(fixture, eventId: secret, now: regressionNow) == nil)
variant = fixture
variant.timestamp = regressionNow.addingTimeInterval(-86401)
assert(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow) == nil)
variant.timestamp = regressionNow.addingTimeInterval(301)
assert(sanitizedNativeCrash(variant, eventId: regressionEventId, now: regressionNow) == nil)

let logPattern = "^nyx-crash: raw report discarded reason=(oversize|unparseable) bytes=([0-9]+|unknown)$"
let logByteCounts: [Int?] = [nil, 0, rawFixtureData.count, crashRawReportMaxBytes + 1, Int.max, -1]
for reason in [CrashDiscardReason.oversize, .unparseable] {
  for count in logByteCounts {
    let line = crashDiscardLogLine(reason: reason, byteCount: count)
    assert(crashMatches(line, logPattern))
    let expectedBytes = count.flatMap { $0 >= 0 ? String($0) : nil } ?? "unknown"
    assert(line == "nyx-crash: raw report discarded reason=\(reason.rawValue) bytes=\(expectedBytes)")
    for value in privateSentinels { assert(!line.contains(value)) }
  }
}
assert(crashDiscardLogLine(reason: .oversize, byteCount: crashRawReportMaxBytes + 1)
  == "nyx-crash: raw report discarded reason=oversize bytes=4194305")
assert(crashDiscardLogLine(reason: .unparseable, byteCount: nil)
  == "nyx-crash: raw report discarded reason=unparseable bytes=unknown")
let formatter = ISO8601DateFormatter()
formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
let base: [String: Any] = [
  "schema_version": 1, "event_id": UUID().uuidString, "captured_at": formatter.string(from: Date()),
  "platform": "ios", "kind": "js_fatal", "exception_class": "Error", "app_version": "1.2.3",
  "app_build": "123", "os_version": "18.1",
  "js_frames": [["asset": "main.jsbundle", "line": 1, "column": 2]], "native_images": [],
  "native_frames": [],
]
assert(sanitizedFatalJs(base) != nil)
for key in ["reason", "registers", "message", "customData", "absolutePath", "extra"] {
  var bad = base
  bad[key] = secret
  assert(sanitizedFatalJs(bad) == nil)
}
var badPath = base
badPath["js_frames"] = [["asset": "/Users/private/main.jsbundle", "line": 1, "column": 2]]
assert(sanitizedFatalJs(badPath) == nil)
var native = base
native["kind"] = "native_crash"
native["exception_class"] = "Unknown"
native["native_signal"] = "SIGABRT"
native["js_frames"] = []
native["native_images"] = [
  ["uuid": UUID().uuidString, "name": "NyxIDMobile", "size": 4096, "registers": secret]
]
native["native_frames"] = [["image": 0, "offset": 32, "function": secret]]
for key in ["reason", "registers", "message", "customData", "absolutePath"] { native[key] = secret }
let output = sanitizedNativeRecord(native)!
assert(Set(output.keys) == nativeKeys)
let bytes = String(data: try JSONSerialization.data(withJSONObject: output), encoding: .utf8)!
for value in [
  "SECRET_SENTINEL", "Bearer", "person@", "/Users", "eyJ", "reason", "registers", "message",
  "customData", "function",
] { assert(!bytes.contains(value)) }
print("Generated native crash regression fixture: \(rawFixtureData.count) bytes (40 threads x 64 frames, 600 images)")
print("Native crash size decision, mapping, discard log, JS fatal rejection and native allowlist proofs passed")
