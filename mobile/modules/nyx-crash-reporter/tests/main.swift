import Foundation

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
assert(
  Set(output.keys)
    == Set([
      "schema_version", "event_id", "captured_at", "platform", "kind", "exception_class",
      "native_signal", "app_version", "app_build", "os_version", "js_frames", "native_images",
      "native_frames",
    ]))
let bytes = String(data: try JSONSerialization.data(withJSONObject: output), encoding: .utf8)!
for value in [
  "SECRET_SENTINEL", "Bearer", "person@", "/Users", "eyJ", "reason", "registers", "message",
  "customData", "function",
] { assert(!bytes.contains(value)) }
print("Native crash decision, JS fatal rejection and native allowlist proofs passed")
