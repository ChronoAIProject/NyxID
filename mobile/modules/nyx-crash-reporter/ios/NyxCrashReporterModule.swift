import CrashReporter
import Darwin
import ExpoModulesCore
import Foundation

public class NyxCrashReporterModule: Module {
  private struct ReporterPendingRawSource: PendingRawSource {
    let reporter: PLCrashReporter
    let modificationDateValue: Date?
    var hasPending: Bool { reporter.hasPendingCrashReport() }
    var modificationDate: Date? { modificationDateValue }
    func loadData() -> Data? { reporter.loadPendingCrashReportData() }
    func purge() { _ = reporter.purgePendingCrashReport() }
  }

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
  @discardableResult private func purgePendingRaw() -> Bool {
    if let reporter {
      let hadPending = reporter.hasPendingCrashReport()
      _ = reporter.purgePendingCrashReport()
      return hadPending && !reporter.hasPendingCrashReport()
    } else {
      let raw = directory.appendingPathComponent("raw", isDirectory: true)
      let files = (try? FileManager.default.contentsOfDirectory(
        at: raw, includingPropertiesForKeys: nil)) ?? []
      let pendingFiles = (FileManager.default.enumerator(
        at: raw, includingPropertiesForKeys: [.isRegularFileKey]))?.compactMap { item -> URL? in
          guard let url = item as? URL,
            (try? url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
          else { return nil }
          return url
        } ?? []
      for file in files { try? FileManager.default.removeItem(at: file) }
      return pendingFiles.contains { !FileManager.default.fileExists(atPath: $0.path) }
    }
  }
  private func purgePendingRawAndLog(_ reason: CrashDiscardReason) {
    if purgePendingRaw() { logRaw(reason, byteCount: nil) }
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
    purgePendingRawAndLog(reporter == nil ? .reporterUnavailable : .unauthorized)
  }
  private func trim() {
    purgeInterruptedCrashWrites(in: directory)
    let now = Date()
    var retained: [(URL, Int)] = []
    for file in recordFiles() {
      guard let data = try? Data(contentsOf: file), data.count <= 32 * 1024,
        let wrapper = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
        let record = wrapper["report"] as? [String: Any],
        crashRecordIsFresh(capturedAt: record["captured_at"], now: now)
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
  private func logRaw(_ reason: CrashDiscardReason, byteCount: Int?) {
    NSLog("%@", crashDiscardLogLine(reason: reason, byteCount: byteCount))
  }
  private func rawAuthorityEpoch() -> String? {
    guard let authority = try? Data(contentsOf: rawAuthorityURL),
      let value = String(data: authority, encoding: .utf8), !value.isEmpty
    else { return nil }
    return value
  }
  private func pendingRawModificationDate(_ reporter: PLCrashReporter) -> Date? {
    guard let path = reporter.crashReportPath(),
      let attributes = try? FileManager.default.attributesOfItem(atPath: path),
      let modified = attributes[.modificationDate] as? Date else { return nil }
    return modified
  }
  private func rawEpoch(_ crash: PLCrashReport) -> String? {
    guard let custom = crash.customData,
      let token = (try? JSONSerialization.jsonObject(with: custom)) as? [String: String]
    else { return nil }
    return token["e"]
  }
  private func rawJsEventId(_ crash: PLCrashReport) -> String? {
    guard let custom = crash.customData,
      let token = (try? JSONSerialization.jsonObject(with: custom)) as? [String: String]
    else { return nil }
    return token["j"]
  }
  /// Never access exceptionInfo, registers, processInfo, machineInfo or symbols.
  private func parsePendingRaw(_ data: Data) -> ParsedPendingRaw? {
    guard let crash = try? PLCrashReport(data: data),
      let signal = crash.signalInfo,
      let application = crash.applicationInfo,
      let system = crash.systemInfo,
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
        return ParsedNativeCrash.Image(
          uuid: $0.imageUUID, name: name, baseAddress: $0.imageBaseAddress, size: $0.imageSize)
      },
      signalName: signal.name, faultAddress: signal.address,
      appMarketingVersion: application.applicationMarketingVersion,
      appBuild: application.applicationVersion,
      osVersion: osVersion, timestamp: system.timestamp)
    return ParsedPendingRaw(
      crash: parsed, reportEpoch: rawEpoch(crash), jsEventId: rawJsEventId(crash))
  }

  private func consumePendingRawFromReporter(
    reporter: PLCrashReporter,
    bound: [String: String],
    now: Date
  ) -> [String: Any]? {
    let source = ReporterPendingRawSource(
      reporter: reporter, modificationDateValue: pendingRawModificationDate(reporter))
    return consumePendingRaw(
      source: source,
      bindingEpoch: bound["epochId"],
      authorityEpoch: rawAuthorityEpoch(),
      now: now,
      parse: { [weak self] data in self?.parsePendingRaw(data) },
      persist: { [weak self] record in
        guard let self else { return }
        try? self.persist(record, bound: bound)
      })
  }

  /// Module initialization converts an authorized report before purging it.
  private func initialRawPurge() {
    guard let reporter else {
      purgePendingRawAndLog(.reporterUnavailable)
      return
    }
    guard let bound = binding() else {
      purgePendingRawAndLog(.unauthorized)
      return
    }
    _ = consumePendingRawFromReporter(reporter: reporter, bound: bound, now: Date())
  }
  /// All conversion is invoked from JS, with no AppDelegate/SceneDelegate hooks.
  private func collect() -> [String] {
    guard let bound = binding() else {
      disarm()
      return []
    }
    if let reporter {
      _ = consumePendingRawFromReporter(reporter: reporter, bound: bound, now: Date())
    } else {
      purgePendingRawAndLog(.reporterUnavailable)
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
