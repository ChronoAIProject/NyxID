/**
 * Plain-language phrasing for recorded tool calls. Activity labels are
 * metadata-only tool names (`api-github-4__get_issue`, `nyx__search_tools`),
 * so everything here is derived from the name alone.
 */

export interface ToolActivityPhrase {
  /** Task form for the action list: "Read issue on GitHub". */
  readonly action: string;
  /** Live form while the call runs: "Reading issue on GitHub". */
  readonly progress: string;
}

const META_TOOLS: Readonly<Record<string, readonly [string, string]>> = {
  search_tools: ["Search available tools", "Searching available tools"],
  call_tool: ["Call a tool", "Calling a tool"],
  discover_services: ["Discover services", "Discovering services"],
  list_connected_services: ["Check connected services", "Checking connected services"],
  connect_service: ["Prepare a connection link", "Preparing a connection link"],
  wait_for_connection: ["Wait for the connection", "Waiting for the connection"],
  oracle_ask: ["Ask Oracle", "Asking Oracle"],
  oracle_result: ["Check the Oracle answer", "Checking the Oracle answer"],
  oracle_pools: ["List Oracle pools", "Listing Oracle pools"],
  ssh_exec: ["Run a remote command", "Running a remote command"],
  ssh_list_services: ["List SSH services", "Listing SSH services"],
  remember: ["Save a memory note", "Saving a memory note"],
  forget: ["Remove a memory note", "Removing a memory note"],
  check: ["Run a check", "Running a check"],
  message_subagent: ["Message a specialist", "Messaging a specialist"],
};

const VERBS: Readonly<Record<string, readonly [string, string]>> = {
  get: ["Read", "Reading"],
  read: ["Read", "Reading"],
  fetch: ["Fetch", "Fetching"],
  list: ["List", "Listing"],
  search: ["Search", "Searching"],
  find: ["Find", "Finding"],
  query: ["Query", "Querying"],
  create: ["Create", "Creating"],
  add: ["Add", "Adding"],
  append: ["Append", "Appending"],
  update: ["Update", "Updating"],
  edit: ["Edit", "Editing"],
  set: ["Set", "Setting"],
  patch: ["Update", "Updating"],
  delete: ["Delete", "Deleting"],
  remove: ["Remove", "Removing"],
  clear: ["Clear", "Clearing"],
  send: ["Send", "Sending"],
  post: ["Post", "Posting"],
  reply: ["Reply to", "Replying to"],
  forward: ["Forward", "Forwarding"],
  compose: ["Compose", "Composing"],
  submit: ["Submit", "Submitting"],
  upload: ["Upload", "Uploading"],
  download: ["Download", "Downloading"],
  export: ["Export", "Exporting"],
  copy: ["Copy", "Copying"],
  run: ["Run", "Running"],
  execute: ["Run", "Running"],
  start: ["Start", "Starting"],
  spawn: ["Start", "Starting"],
  stop: ["Stop", "Stopping"],
  pause: ["Pause", "Pausing"],
  cancel: ["Cancel", "Cancelling"],
  check: ["Check", "Checking"],
  scrape: ["Scrape", "Scraping"],
  map: ["Map", "Mapping"],
  request: ["Request", "Requesting"],
  approve: ["Approve", "Approving"],
  grant: ["Grant", "Granting"],
  bind: ["Bind", "Binding"],
};

const SERVICE_NAMES: Readonly<Record<string, string>> = {
  github: "GitHub",
  openai: "OpenAI",
  openrouter: "OpenRouter",
  deepseek: "DeepSeek",
  elevenlabs: "ElevenLabs",
  twitter: "X",
  x: "X",
  gmail: "Gmail",
  "google-workspace": "Google Workspace",
  "google-calendar": "Google Calendar",
  "zoho-mail": "Zoho Mail",
  "ornn-api": "Ornn",
  nyx: "NyxAgent",
};

const MAX_WORDS_CHARS = 60;

function words(parts: readonly string[]): string {
  const text = parts.join(" ").trim();
  return text.length <= MAX_WORDS_CHARS ? text : `${text.slice(0, MAX_WORDS_CHARS - 1).trimEnd()}…`;
}

/** `api-github-4` -> `GitHub`, `api-google-workspace-2` -> `Google Workspace`. */
export function toolServiceName(slug: string): string {
  const bare = slug
    .trim()
    .toLowerCase()
    .replace(/^(api|llm|tools)-/, "")
    .replace(/-\d+$/, "");
  const known = SERVICE_NAMES[bare];
  if (known) return known;
  return bare
    .split(/[-_]/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function operationPhrase(operation: string): readonly [string, string] {
  const tokens = operation.toLowerCase().split(/[_\-\s]+/).filter(Boolean);
  const at = tokens.findIndex((token) => VERBS[token]);
  const verb = at < 0 ? undefined : VERBS[tokens[at] ?? ""];
  if (!verb) {
    const object = words(tokens);
    return object ? [`Run ${object}`, `Running ${object}`] : ["Run a tool", "Running a tool"];
  }
  const [action, progress] = verb;
  const object = words([...tokens.slice(0, at), ...tokens.slice(at + 1)]);
  return object ? [`${action} ${object}`, `${progress} ${object}`] : [action, progress];
}

export function describeToolActivity(name: string): ToolActivityPhrase {
  const trimmed = name.trim();
  const split = trimmed.indexOf("__");
  const prefix = split > 0 ? trimmed.slice(0, split) : "";
  const operation = split > 0 ? trimmed.slice(split + 2) : trimmed;
  if (!prefix || prefix === "nyx" || prefix === "nyxid") {
    const meta = META_TOOLS[operation.toLowerCase()];
    const [action, progress] = meta ?? operationPhrase(operation);
    return { action, progress };
  }
  const [action, progress] = operationPhrase(operation);
  const service = toolServiceName(prefix);
  return service
    ? { action: `${action} on ${service}`, progress: `${progress} on ${service}` }
    : { action, progress };
}
