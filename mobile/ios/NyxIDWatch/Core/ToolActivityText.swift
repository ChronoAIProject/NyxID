import Foundation

/// Human words for a tool call, from its identifier alone (activities never
/// carry arguments). Port of `describeToolActivity` in
/// `mobile/src/features/assistant/assistantActivityText.ts`; keep in step.
enum ToolActivityText {
  /// "api-google-calendar__list_events" → "Listing events on Google Calendar".
  static func running(_ label: String) -> String {
    forms(label)?.running ?? "Working"
  }

  struct Forms: Equatable {
    let running: String
    let done: String
  }

  static func forms(_ label: String) -> Forms? {
    if let meta = meta[label] { return meta }
    guard let split = label.range(of: "__"), split.lowerBound > label.startIndex,
      split.upperBound < label.endIndex
    else { return nil }
    let service = String(label[..<split.lowerBound])
    let operation = operationForms(String(label[split.upperBound...]))
    if service == "nyx" || service == "nyxid" { return operation }
    let place = " on \(serviceName(service))"
    return Forms(running: operation.running + place, done: operation.done + place)
  }

  private static let meta: [String: Forms] = [
    "nyx__search_tools": Forms(running: "Searching for tools", done: "Searched for tools"),
    "nyx__discover_services": Forms(running: "Browsing the service catalog", done: "Browsed the service catalog"),
    "nyx__list_connected_services": Forms(running: "Checking connected services", done: "Checked connected services"),
    "nyx__connect_service": Forms(running: "Preparing a connection link", done: "Prepared a connection link"),
    "nyx__wait_for_connection": Forms(running: "Waiting for the connection", done: "Waited for the connection"),
    "nyx__call_tool": Forms(running: "Calling a tool", done: "Called a tool"),
    "nyx__ssh_exec": Forms(running: "Running an SSH command", done: "Ran an SSH command"),
    "nyx__ssh_list_services": Forms(running: "Listing SSH services", done: "Listed SSH services"),
  ]

  private static let verbs: [String: (String, String)] = [
    "get": ("Reading", "Read"), "read": ("Reading", "Read"), "fetch": ("Fetching", "Fetched"),
    "download": ("Downloading", "Downloaded"), "export": ("Exporting", "Exported"),
    "list": ("Listing", "Listed"), "search": ("Searching", "Searched"), "query": ("Querying", "Queried"),
    "find": ("Finding", "Found"), "check": ("Checking", "Checked"), "create": ("Creating", "Created"),
    "add": ("Adding", "Added"), "append": ("Appending", "Appended"), "upload": ("Uploading", "Uploaded"),
    "post": ("Posting", "Posted"), "submit": ("Submitting", "Submitted"), "send": ("Sending", "Sent"),
    "reply": ("Replying to", "Replied to"), "compose": ("Composing", "Composed"),
    "forward": ("Forwarding", "Forwarded"), "update": ("Updating", "Updated"), "edit": ("Editing", "Edited"),
    "patch": ("Updating", "Updated"), "put": ("Updating", "Updated"), "set": ("Setting", "Set"),
    "rename": ("Renaming", "Renamed"), "move": ("Moving", "Moved"), "copy": ("Copying", "Copied"),
    "delete": ("Deleting", "Deleted"), "remove": ("Removing", "Removed"), "clear": ("Clearing", "Cleared"),
    "cancel": ("Cancelling", "Cancelled"), "run": ("Running", "Ran"), "execute": ("Running", "Ran"),
    "invoke": ("Invoking", "Invoked"), "call": ("Calling", "Called"), "start": ("Starting", "Started"),
    "stop": ("Stopping", "Stopped"), "generate": ("Generating", "Generated"),
    "connect": ("Connecting", "Connected"), "grant": ("Granting", "Granted"),
    "schedule": ("Scheduling", "Scheduled"), "remember": ("Remembering", "Remembered"),
    "forget": ("Forgetting", "Forgot"),
  ]

  private static let providers: [String: String] = [
    "openai": "OpenAI", "anthropic": "Anthropic", "google-ai": "Google AI", "mistral": "Mistral",
    "cohere": "Cohere", "deepseek": "DeepSeek", "openai-codex": "Codex", "twitter": "X (Twitter)",
    "google": "Google", "github": "GitHub", "cloudflare": "Cloudflare", "supabase": "Supabase",
    "supabase-management": "Supabase Management", "railway": "Railway", "facebook": "Facebook",
    "instagram": "Instagram", "linkedin": "LinkedIn", "discord": "Discord", "spotify": "Spotify",
    "slack": "Slack", "microsoft": "Microsoft", "apple": "Apple", "tiktok": "TikTok", "twitch": "Twitch",
    "reddit": "Reddit", "telegram": "Telegram", "telegram-bot": "Telegram Bot", "lark": "Lark",
    "feishu": "Feishu",
  ]

  private static let noise: Set<String> = ["api", "v1", "v2", "v3"]

  /// "api-github-4" → "GitHub"; "api-google-workspace-2" → "Google Workspace".
  static func serviceName(_ slug: String) -> String {
    var base = slug
    for prefix in ["api-", "llm-", "tools-"] where base.hasPrefix(prefix) {
      base = String(base.dropFirst(prefix.count))
      break
    }
    if let range = base.range(of: #"-\d+$"#, options: .regularExpression) {
      base.removeSubrange(range)
    }
    if let known = providers[base] { return known }
    return base.split(separator: "-").map { $0.prefix(1).uppercased() + $0.dropFirst() }.joined(separator: " ")
  }

  private static func operationForms(_ operation: String) -> Forms {
    let words = operation.lowercased()
      .split(whereSeparator: { "_-. ".contains($0) })
      .map(String.init)
      .filter { !$0.isEmpty && !noise.contains($0) }
    guard let index = words.firstIndex(where: { verbs[$0] != nil }), let verb = verbs[words[index]] else {
      let object = words.isEmpty ? operation : words.joined(separator: " ")
      return Forms(running: "Using \(object)", done: "Used \(object)")
    }
    let object = words.enumerated().filter { $0.offset != index }.map(\.element).joined(separator: " ")
    let tail = object.isEmpty ? "" : " \(object)"
    return Forms(running: verb.0 + tail, done: verb.1 + tail)
  }
}
