import CrashReporter
import Darwin
import ExpoModulesCore
import Foundation

public class NyxCrashReporterModule: Module {
  private let lock = NSRecursiveLock()
  private var enabled = false
  private lazy var directory: URL = {
    let url = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
      .appendingPathComponent("nyx-crash", isDirectory: true)
    try? FileManager.default.createDirectory(
      at: url, withIntermediateDirectories: true,
      attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
    var resource = url
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try? resource.setResourceValues(values)
    return url
  }()
  private lazy var reporter: PLCrashReporter = {
    let raw = directory.appendingPathComponent("raw", isDirectory: true)
    try? FileManager.default.createDirectory(
      at: raw, withIntermediateDirectories: true,
      attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
    let config = PLCrashReporterConfig(
      signalHandlerType: .BSD, symbolicationStrategy: [],
      shouldRegisterUncaughtExceptionHandler: false, basePath: raw.path, maxReportBytes: 256 * 1024)!
    return PLCrashReporter(configuration: config)!
  }()
  private var bindingURL: URL { directory.appendingPathComponent("binding.json") }
  private func binding() -> [String: String]? {
    guard let data = try? Data(contentsOf: bindingURL), data.count < 4096,
      let value = (try? JSONSerialization.jsonObject(with: data)) as? [String: String],
      Set(value.keys) == Set(["accountId", "revision", "epochId"]),
      value.values.allSatisfy({ UUID(uuidString: $0) != nil })
    else { return nil }
    return value
  }
  private func write(_ data: Data, to url: URL) throws {
    let temporary = url.appendingPathExtension("tmp")
    try data.write(
      to: temporary, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    let fd = Darwin.open(temporary.path, O_RDONLY)
    guard fd >= 0 else { throw NSError(domain: "NyxCrash", code: 1) }
    guard fsync(fd) == 0 else {
      close(fd)
      throw NSError(domain: "NyxCrash", code: 2)
    }
    close(fd)
    guard rename(temporary.path, url.path) == 0 else { throw NSError(domain: "NyxCrash", code: 3) }
    let directoryFd = Darwin.open(directory.path, O_RDONLY)
    if directoryFd >= 0 {
      _ = fsync(directoryFd)
      close(directoryFd)
    }
  }
  private func recordFiles() -> [URL] {
    ((try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil))
      ?? []).filter { $0.lastPathComponent.hasPrefix("event-") && $0.pathExtension == "json" }
      .sorted { $0.lastPathComponent < $1.lastPathComponent }
  }
  private func purgeRecords() {
    for file in recordFiles() { try? FileManager.default.removeItem(at: file) }
  }
  private func disarm() {
    reporter.customData = nil
    try? FileManager.default.removeItem(at: bindingURL)
    purgeRecords()
    purgeInterruptedCrashWrites(in: directory)
    // Explicit purge point 1: no current capture authority; raw data is deleted unread.
    _ = reporter.purgePendingCrashReport()
  }
  private func trim() {
    purgeInterruptedCrashWrites(in: directory)
    let now = Date().timeIntervalSince1970
    var retained: [(URL, Int)] = []
    for file in recordFiles() {
      guard let data = try? Data(contentsOf: file), data.count <= 32 * 1024,
        let wrapper = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
        let captured = wrapper["storedAt"] as? Double,
        now - captured <= 86400, captured <= now + 300
      else {
        try? FileManager.default.removeItem(at: file)
        continue
      }
      retained.append((file, data.count))
    }
    while retained.count > 3 || retained.reduce(0, { $0 + $1.1 }) > 256 * 1024 {
      let file = retained.removeFirst().0
      try? FileManager.default.removeItem(at: file)
    }
  }
  private func persist(_ record: [String: Any], bound: [String: String]) throws {
    let wrapper: [String: Any] = [
      "binding": bound, "report": record, "storedAt": Date().timeIntervalSince1970,
    ]
    let data = try JSONSerialization.data(withJSONObject: wrapper, options: [.sortedKeys])
    guard data.count <= 32 * 1024, let event = record["event_id"] as? String,
      UUID(uuidString: event) != nil
    else { return }
    try write(
      data,
      to: directory.appendingPathComponent(
        "event-\(Int(Date().timeIntervalSince1970*1000))-\(event).json"))
    trim()
  }
  /// All processing is invoked from JS, with no AppDelegate/SceneDelegate hooks.
  private func collect() -> [String] {
    defer { _ = reporter.purgePendingCrashReport() }  // Explicit purge point 2: success, mismatch, corrupt, oversized and JS-duplicate paths.
    guard let bound = binding() else {
      disarm()
      return []
    }
    if reporter.hasPendingCrashReport() {
      if let data = reporter.loadPendingCrashReportData(), data.count <= 256 * 1024,
        let crash = try? PLCrashReport(data: data),
        let custom = crash.customData,
        let token = (try? JSONSerialization.jsonObject(with: custom)) as? [String: String],
        pendingCrashDecision(
          bindingEpoch: bound["epochId"], reportEpoch: token["e"], jsEventId: token["j"])
          == .native,
        let record = sanitize(crash)
      {
        try? persist(record, bound: bound)
      }
      _ = reporter.purgePendingCrashReport()  // Purge before returning any sanitized data to JS.
    }
    trim()
    return recordFiles().compactMap { file in
      guard let data = try? Data(contentsOf: file),
        let wrapper = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
        let saved = wrapper["binding"] as? [String: String], saved == bound,
        let record = wrapper["report"] as? [String: Any],
        let output = try? JSONSerialization.data(withJSONObject: [
          "binding": saved, "report": record,
        ])
      else {
        try? FileManager.default.removeItem(at: file)
        return nil
      }
      return String(data: output, encoding: .utf8)
    }
  }
  /// Never access exceptionInfo, registers, processInfo, machineInfo or symbols.
  private func sanitize(_ crash: PLCrashReport) -> [String: Any]? {
    guard let thread = (crash.threads as? [PLCrashReportThreadInfo])?.first(where: { $0.crashed })
    else { return nil }
    let allImages = (crash.images as? [PLCrashReportBinaryImageInfo]) ?? []
    var selected: [PLCrashReportBinaryImageInfo] = []
    var images: [[String: Any]] = []
    var frames: [[String: Any]] = []
    for frame in (thread.stackFrames as? [PLCrashReportStackFrameInfo] ?? []).prefix(64) {
      guard
        let image = allImages.first(where: {
          frame.instructionPointer >= $0.imageBaseAddress
            && frame.instructionPointer - $0.imageBaseAddress < $0.imageSize
        }),
        let rawUuid = image.imageUUID
      else {
        frames.append(["image": NSNull(), "offset": 0])
        continue
      }
      let compact = rawUuid.replacingOccurrences(of: "-", with: "")
      guard compact.count == 32 else { continue }
      let uuidText =
        "\(compact.prefix(8))-\(compact.dropFirst(8).prefix(4))-\(compact.dropFirst(12).prefix(4))-\(compact.dropFirst(16).prefix(4))-\(compact.suffix(12))"
      guard UUID(uuidString: uuidText) != nil else { continue }
      let name = URL(fileURLWithPath: image.imageName).lastPathComponent
      guard crashMatches(name, "^[A-Za-z0-9_.+-]{1,64}$") else {
        frames.append(["image": NSNull(), "offset": 0])
        continue
      }
      var index = selected.firstIndex(where: { $0.imageBaseAddress == image.imageBaseAddress })
      if index == nil && selected.count < 16 {
        index = selected.count
        selected.append(image)
        images.append(["uuid": uuidText.lowercased(), "name": name, "size": image.imageSize])
      }
      if let index {
        frames.append(["image": index, "offset": frame.instructionPointer - image.imageBaseAddress])
      } else {
        frames.append(["image": NSNull(), "offset": 0])
      }
    }
    let version = crash.applicationInfo.applicationMarketingVersion ?? "0"
    let build = crash.applicationInfo.applicationVersion ?? "0"
    guard crashMatches(version, "^[0-9A-Za-z.+-]{1,32}$"),
      crashMatches(build, "^[0-9A-Za-z.+-]{1,64}$")
    else { return nil }
    let os = crash.systemInfo.operatingSystemVersion.split(separator: ".").prefix(2).joined(
      separator: ".")
    guard crashMatches(os, "^[0-9]{1,3}(\\.[0-9]{1,3})?$") else { return nil }
    let signal = crash.signalInfo.name ?? "Other"
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return sanitizedNativeRecord([
      "schema_version": 1, "event_id": UUID().uuidString.lowercased(),
      "captured_at": formatter.string(from: crash.systemInfo.timestamp), "platform": "ios",
      "kind": "native_crash", "exception_class": "Unknown",
      "native_signal": ["SIGSEGV", "SIGBUS", "SIGABRT", "SIGILL", "SIGFPE", "SIGTRAP", "SIGSYS"]
        .contains(signal) ? signal : "Other", "app_version": version, "app_build": build,
      "os_version": os, "js_frames": [], "native_images": images, "native_frames": frames,
    ])
  }
  public func definition() -> ModuleDefinition {
    Name("NyxCrashReporter")
    Function("getAppIdentity") {
      [
        "osVersion":
          "\(ProcessInfo.processInfo.operatingSystemVersion.majorVersion).\(ProcessInfo.processInfo.operatingSystemVersion.minorVersion)",
        "version": Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
          ?? "0",
        "build": Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "0",
      ]
    }
    Function("getBinding") { () -> [String: String]? in
      self.lock.lock()
      defer { self.lock.unlock() }
      return self.binding()
    }
    Function("arm") { (value: [String: String]) in
      self.lock.lock()
      defer { self.lock.unlock() }
      guard Set(value.keys) == Set(["accountId", "revision", "epochId"]),
        value.values.allSatisfy({ UUID(uuidString: $0) != nil })
      else {
        self.disarm()
        return
      }
      if self.binding() != value { self.disarm() }
      try self.write(JSONSerialization.data(withJSONObject: value), to: self.bindingURL)
      self.reporter.customData = try JSONSerialization.data(withJSONObject: ["e": value["epochId"]!]
      )
      if !self.enabled { self.enabled = self.reporter.enable() }
      if !self.enabled { self.disarm() }
    }
    Function("disarm") {
      self.lock.lock()
      defer { self.lock.unlock() }
      self.disarm()
    }
    Function("recordFatalJs") { (json: String) in
      self.lock.lock()
      defer { self.lock.unlock() }
      guard let bound = self.binding(), let data = json.data(using: .utf8), data.count <= 32 * 1024,
        let object = try? JSONSerialization.jsonObject(with: data),
        let record = sanitizedFatalJs(object)
      else { return }
      try self.persist(record, bound: bound)
      self.reporter.customData = try JSONSerialization.data(withJSONObject: [
        "e": bound["epochId"]!, "j": record["event_id"] as! String,
      ])
    }
    Function("collectPending") { () -> [String] in
      self.lock.lock()
      defer { self.lock.unlock() }
      return self.collect()
    }
    Function("removeRecords") { (ids: [String]) in
      self.lock.lock()
      defer { self.lock.unlock() }
      for file in self.recordFiles() {
        if ids.contains(where: {
          UUID(uuidString: $0) != nil && file.lastPathComponent.hasSuffix("-\($0).json")
        }) {
          try? FileManager.default.removeItem(at: file)
        }
      }
    }
  }
}
