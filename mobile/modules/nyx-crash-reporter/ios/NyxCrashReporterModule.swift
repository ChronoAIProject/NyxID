import CrashReporter
import Darwin
import ExpoModulesCore
import Foundation

public class NyxCrashReporterModule: Module {
  private let lock = NSRecursiveLock()
  private var enabled = false
  private var armed = false
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
  private lazy var reporter: PLCrashReporter? = {
    let raw = directory.appendingPathComponent("raw", isDirectory: true)
    try? FileManager.default.createDirectory(
      at: raw, withIntermediateDirectories: true,
      attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
    guard let config = PLCrashReporterConfig(
      signalHandlerType: .BSD, symbolicationStrategy: [],
      shouldRegisterUncaughtExceptionHandler: false, basePath: raw.path,
      maxReportBytes: UInt(crashRawReportMaxBytes)) else { return nil }
    return PLCrashReporter(configuration: config)
  }()
  private var rawAuthorityURL: URL { directory.appendingPathComponent("raw-authority.json") }
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
  private func purgePendingRaw() {
    if let reporter { _ = reporter.purgePendingCrashReport() }
    else {
      let raw = directory.appendingPathComponent("raw", isDirectory: true)
      for file in (try? FileManager.default.contentsOfDirectory(at: raw, includingPropertiesForKeys: nil)) ?? [] { try? FileManager.default.removeItem(at: file) }
    }
  }
  private func pause() {
    armed = false
    reporter?.customData = nil
    // Convert a capture-time-authorized raw report locally, then purge it even
    // when the network is offline. Only bounded sanitized records survive pause.
    _ = collect()
    try? FileManager.default.removeItem(at: rawAuthorityURL)
  }
  private func disarm() {
    armed = false
    reporter?.customData = nil
    try? FileManager.default.removeItem(at: rawAuthorityURL)
    try? FileManager.default.removeItem(at: bindingURL)
    purgeRecords()
    purgeInterruptedCrashWrites(in: directory)
    // Explicit purge point 1: no current capture authority; raw data is deleted unread.
    purgePendingRaw()
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
  private func loadPendingRaw(_ reporter: PLCrashReporter) -> PLCrashReport? {
    let data = reporter.loadPendingCrashReportData()
    switch rawCrashReportDecision(byteCount: data?.count ?? 0) {
    case .discard(let reason):
      let line = crashDiscardLogLine(reason: reason, byteCount: data?.count)
      NSLog("%@", line)
      return nil
    case .parse:
      break
    }
    guard let data, let crash = try? PLCrashReport(data: data) else {
      let line = crashDiscardLogLine(reason: .unparseable, byteCount: data?.count)
      NSLog("%@", line)
      return nil
    }
    return crash
  }
  private func pendingRawAge() -> TimeInterval? {
    let raw = directory.appendingPathComponent("raw/live_report.plcrash")
    guard let attributes = try? FileManager.default.attributesOfItem(atPath: raw.path),
      let modified = attributes[.modificationDate] as? Date else { return nil }
    return Date().timeIntervalSince(modified)
  }
  private func rawEpoch(_ crash: PLCrashReport) -> String? {
    guard let custom = crash.customData,
      let token = (try? JSONSerialization.jsonObject(with: custom)) as? [String: String]
    else { return nil }
    return token["e"]
  }
  /// Module initialization only purges. Enabling and arming stay JS-driven.
  private func initialRawPurge() {
    guard let reporter else { purgePendingRaw(); return }
    guard reporter.hasPendingCrashReport() else { return }
    guard let bound = binding(), let epoch = bound["epochId"],
      let authority = try? Data(contentsOf: rawAuthorityURL),
      String(data: authority, encoding: .utf8) == epoch,
      initialRawReportDecision(bindingEpoch: epoch, reportEpoch: epoch, fileAgeSeconds: pendingRawAge()) == .retain
    else { purgePendingRaw(); return } // No binding/authority or expired: delete unread.
    guard let crash = loadPendingRaw(reporter),
      initialRawReportDecision(bindingEpoch: epoch, reportEpoch: rawEpoch(crash), fileAgeSeconds: pendingRawAge()) == .retain
    else { purgePendingRaw(); return }
  }
  /// All processing is invoked from JS, with no AppDelegate/SceneDelegate hooks.
  private func collect() -> [String] {
    defer { purgePendingRaw() }  // Explicit purge point 2: success, mismatch, corrupt, oversized and JS-duplicate paths.
    guard let bound = binding() else {
      disarm()
      return []
    }
    if let reporter, reporter.hasPendingCrashReport() {
      if initialRawReportDecision(bindingEpoch: bound["epochId"], reportEpoch: bound["epochId"], fileAgeSeconds: pendingRawAge()) == .retain,
        let crash = loadPendingRaw(reporter),
        let custom = crash.customData,
        let token = (try? JSONSerialization.jsonObject(with: custom)) as? [String: String],
        pendingCrashDecision(
          bindingEpoch: bound["epochId"], reportEpoch: token["e"], jsEventId: token["j"])
          == .native,
        let record = sanitize(crash)
      {
        try? persist(record, bound: bound)
      }
      purgePendingRaw()  // Purge before returning any sanitized data to JS.
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
    guard let signal = crash.signalInfo, let application = crash.applicationInfo,
      let system = crash.systemInfo, let timestamp = system.timestamp,
      let osVersion = system.operatingSystemVersion
    else { return nil }
    let parsed = ParsedNativeCrash(
      threads: (crash.threads as? [PLCrashReportThreadInfo] ?? []).map {
        ParsedNativeCrash.Thread(
          crashed: $0.crashed,
          frames: ($0.stackFrames as? [PLCrashReportStackFrameInfo] ?? []).map {
            $0.instructionPointer
          })
      },
      images: (crash.images as? [PLCrashReportBinaryImageInfo] ?? []).compactMap {
        guard let name = $0.imageName else { return nil }
        return
        ParsedNativeCrash.Image(
          uuid: $0.imageUUID, name: name, baseAddress: $0.imageBaseAddress, size: $0.imageSize)
      },
      signalName: signal.name, faultAddress: signal.address,
      appMarketingVersion: application.applicationMarketingVersion,
      appBuild: application.applicationVersion,
      osVersion: osVersion, timestamp: timestamp)
    return sanitizedNativeCrash(parsed, eventId: UUID().uuidString.lowercased(), now: Date())
  }
  public func definition() -> ModuleDefinition {
    Name("NyxCrashReporter")
    OnCreate {
      self.lock.lock()
      defer { self.lock.unlock() }
      self.initialRawPurge()
    }
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
    Function("arm") { (value: [String: String]) -> Bool in
      self.lock.lock()
      defer { self.lock.unlock() }
      guard Set(value.keys) == Set(["accountId", "revision", "epochId"]),
        value.values.allSatisfy({ UUID(uuidString: $0) != nil })
      else {
        self.disarm()
        return false
      }
      guard let reporter = self.reporter else { self.disarm(); return false }
      if self.binding() != value { self.disarm() }
      try self.write(JSONSerialization.data(withJSONObject: value), to: self.bindingURL)
      try self.write(Data((value["epochId"] ?? "").utf8), to: self.rawAuthorityURL)
      reporter.customData = try JSONSerialization.data(withJSONObject: ["e": value["epochId"] ?? ""])
      if !self.enabled { self.enabled = reporter.enable() }
      self.armed = crashCaptureCanArm(reporterAvailable: true, enabled: self.enabled)
      if !self.armed { self.disarm() }
      return self.armed
    }
    Function("pause") {
      self.lock.lock()
      defer { self.lock.unlock() }
      self.pause()
    }
    Function("disarm") {
      self.lock.lock()
      defer { self.lock.unlock() }
      self.disarm()
    }
    Function("recordFatalJs") { (json: String) in
      self.lock.lock()
      defer { self.lock.unlock() }
      guard self.armed, let bound = self.binding(), let data = json.data(using: .utf8), data.count <= 32 * 1024,
        let object = try? JSONSerialization.jsonObject(with: data),
        let record = sanitizedFatalJs(object)
      else { return }
      try self.persist(record, bound: bound)
      self.reporter?.customData = try JSONSerialization.data(withJSONObject: [
        "e": bound["epochId"] ?? "", "j": record["event_id"] as? String ?? "",
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
