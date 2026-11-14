import Foundation

/// Pure policy: inspect only the arming token and JS fatal marker, never exception text/registers.
enum PendingCrashDecision: Equatable { case discard, native }
func pendingCrashDecision(bindingEpoch: String?, reportEpoch: String?, jsEventId: String?) -> PendingCrashDecision {
  guard let bindingEpoch, !bindingEpoch.isEmpty, bindingEpoch == reportEpoch, jsEventId == nil else { return .discard }
  return .native
}
func crashMatches(_ value: String, _ pattern: String) -> Bool {
  value.range(of: pattern, options: .regularExpression) != nil
}
/// Second native boundary for fatal JS. Reject unknown keys, then reconstruct a closed record.
func sanitizedFatalJs(_ value: Any) -> [String: Any]? {
  guard let v = value as? [String: Any], Set(v.keys).isSubset(of: Set(["schema_version","event_id","captured_at","platform","kind","exception_class","app_version","app_build","artifact_id","os_version","runtime","js_frames","native_images","native_frames"])),
    v["schema_version"] as? Int == 1, v["kind"] as? String == "js_fatal", v["platform"] as? String == "ios",
    let event = v["event_id"] as? String, UUID(uuidString: event) != nil,
    let captured = v["captured_at"] as? String, captured.count <= 40,
    let version = v["app_version"] as? String, crashMatches(version, "^[0-9A-Za-z.+-]{1,32}$"),
    let build = v["app_build"] as? String, crashMatches(build, "^[0-9A-Za-z.+-]{1,64}$"),
    let os = v["os_version"] as? String, crashMatches(os, "^[0-9]{1,3}(\\.[0-9]{1,3})?$"),
    let cls = v["exception_class"] as? String, ["Error","TypeError","ReferenceError","RangeError","SyntaxError","EvalError","URIError","AggregateError","ChunkLoadError","NonError","Other","Unknown"].contains(cls),
    let frames = v["js_frames"] as? [[String:Any]], frames.count <= 64,
    let images = v["native_images"] as? [Any], images.isEmpty, let native = v["native_frames"] as? [Any], native.isEmpty else { return nil }
  let formatter = ISO8601DateFormatter(); formatter.formatOptions = [.withInternetDateTime,.withFractionalSeconds]
  guard let date = formatter.date(from: captured), date.timeIntervalSinceNow >= -86400, date.timeIntervalSinceNow <= 300 else { return nil }
  var safeFrames: [[String:Any]] = []
  for frame in frames {
    guard Set(frame.keys) == Set(["asset","line","column"]), let asset = frame["asset"] as? String,
      ["main.jsbundle","index.android.bundle","<native>","<external>"].contains(asset), let line = frame["line"] as? Int, let column = frame["column"] as? Int,
      (0...10000000).contains(line), (0...10000000).contains(column) else { return nil }
    safeFrames.append(["asset":asset,"line":line,"column":column])
  }
  var result: [String:Any] = ["schema_version":1,"event_id":event,"captured_at":captured,"platform":v["platform"] as! String,"kind":"js_fatal","exception_class":cls,"app_version":version,"app_build":build,"os_version":os,"js_frames":safeFrames,"native_images":[],"native_frames":[]]
  if let artifact = v["artifact_id"] as? String { guard UUID(uuidString: artifact) != nil else { return nil }; result["artifact_id"] = artifact }
  if let runtime = v["runtime"] as? [String:Any] {
    guard Set(runtime.keys) == Set(["name","major"]), runtime["name"] as? String == "hermes", let major = runtime["major"] as? Int, (0...65535).contains(major) else { return nil }
    result["runtime"] = ["name":"hermes","major":major]
  } else if v["runtime"] != nil { return nil }
  return result
}

/// Closed native output boundary. Extra parser metadata never enters the record.
func sanitizedNativeRecord(_ input: [String:Any]) -> [String:Any]? {
  let required = ["schema_version","event_id","captured_at","platform","kind","exception_class","native_signal","app_version","app_build","os_version","js_frames","native_images","native_frames"]
  guard required.allSatisfy({input[$0] != nil}), input["schema_version"] as? Int == 1,
    input["platform"] as? String == "ios", input["kind"] as? String == "native_crash", input["exception_class"] as? String == "Unknown",
    let event = input["event_id"] as? String, UUID(uuidString:event) != nil,
    let captured = input["captured_at"] as? String, captured.count <= 40,
    let version = input["app_version"] as? String, crashMatches(version,"^[0-9A-Za-z.+-]{1,32}$"),
    let build = input["app_build"] as? String, crashMatches(build,"^[0-9A-Za-z.+-]{1,64}$"),
    let os = input["os_version"] as? String, crashMatches(os,"^[0-9]{1,3}(\\.[0-9]{1,3})?$"),
    let signal = input["native_signal"] as? String, ["SIGSEGV","SIGBUS","SIGABRT","SIGILL","SIGFPE","SIGTRAP","SIGSYS","Other"].contains(signal),
    let js = input["js_frames"] as? [Any], js.isEmpty,
    let images = input["native_images"] as? [[String:Any]], images.count <= 16,
    let frames = input["native_frames"] as? [[String:Any]], frames.count <= 64 else { return nil }
  var safeImages: [[String:Any]] = [], safeFrames: [[String:Any]] = []
  for image in images {
    guard let uuid = image["uuid"] as? String, UUID(uuidString:uuid) != nil,
      let name = image["name"] as? String, crashMatches(name,"^[A-Za-z0-9_.+-]{1,64}$"),
      let size = image["size"] as? NSNumber, size.uint64Value > 0, size.uint64Value <= UInt64(Int64.max) else { return nil }
    safeImages.append(["uuid":uuid,"name":name,"size":size])
  }
  for frame in frames {
    guard let offset = frame["offset"] as? NSNumber, offset.uint64Value <= UInt64(Int64.max) else { return nil }
    if let index = frame["image"] as? Int {
      guard images.indices.contains(index), let size = images[index]["size"] as? NSNumber, offset.uint64Value < size.uint64Value else { return nil }
      safeFrames.append(["image":index,"offset":offset])
    } else { safeFrames.append(["image":NSNull(),"offset":0]) }
  }
  return ["schema_version":1,"event_id":event,"captured_at":captured,"platform":"ios","kind":"native_crash","exception_class":"Unknown","native_signal":signal,"app_version":version,"app_build":build,"os_version":os,"js_frames":[],"native_images":safeImages,"native_frames":safeFrames]
}
