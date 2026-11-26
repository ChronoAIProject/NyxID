import Foundation
#if canImport(Security)
import Security
#endif

/// The watch's own NyxID session. It is never copied from the iPhone: the
/// watch redeems a one-time login code and owns this refresh token alone.
struct StoredSession: Codable, Equatable, Sendable {
  var apiBaseURL: URL
  var accessToken: String
  var refreshToken: String
  var accessExpiresAt: Date
}

extension StoredSession: CustomStringConvertible, CustomDebugStringConvertible {
  // Never print tokens.
  var description: String { "StoredSession(api: \(apiBaseURL.host ?? "?"), tokens: <redacted>)" }
  var debugDescription: String { description }
}

protocol SessionStorage: Sendable {
  func load() -> StoredSession?
  func save(_ session: StoredSession) throws
  func clear()
}

#if canImport(Security)
/// Keeps the session in the watch keychain, readable only while the watch is
/// unlocked and never synced or restored to another device.
struct KeychainSessionStorage: SessionStorage {
  let service: String
  let account: String

  init(service: String = "dev.nyxid.watch.session", account: String = "primary") {
    self.service = service
    self.account = account
  }

  private var baseQuery: [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
    ]
  }

  func load() -> StoredSession? {
    var query = baseQuery
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var item: CFTypeRef?
    guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
      let data = item as? Data
    else { return nil }
    return try? JSONDecoder().decode(StoredSession.self, from: data)
  }

  func save(_ session: StoredSession) throws {
    let data = try JSONEncoder().encode(session)
    let attributes: [String: Any] = [
      kSecValueData as String: data,
      kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
    ]
    let status = SecItemUpdate(baseQuery as CFDictionary, attributes as CFDictionary)
    if status == errSecItemNotFound {
      var insert = baseQuery
      insert.merge(attributes) { _, new in new }
      let added = SecItemAdd(insert as CFDictionary, nil)
      guard added == errSecSuccess else { throw KeychainError(status: added) }
    } else if status != errSecSuccess {
      throw KeychainError(status: status)
    }
  }

  func clear() {
    SecItemDelete(baseQuery as CFDictionary)
  }
}

struct KeychainError: Error, Equatable {
  let status: OSStatus
}
#endif

/// In-memory storage for tests and previews.
final class MemorySessionStorage: SessionStorage, @unchecked Sendable {
  private let lock = NSLock()
  private var value: StoredSession?

  init(_ value: StoredSession? = nil) {
    self.value = value
  }

  func load() -> StoredSession? {
    lock.lock()
    defer { lock.unlock() }
    return value
  }

  func save(_ session: StoredSession) throws {
    lock.lock()
    value = session
    lock.unlock()
  }

  func clear() {
    lock.lock()
    value = nil
    lock.unlock()
  }
}
