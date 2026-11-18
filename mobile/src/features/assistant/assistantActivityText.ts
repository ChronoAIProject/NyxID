import { translateApp } from "../../i18n/appTranslation";
// Ported from Web tool-activity-text.ts at upstream f69a8718 (PR #1870).
// Keep its identifier-only phrases; native needs only the branding labels.
const PROVIDER_LABELS: Readonly<Record<string, string>> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  "google-ai": "Google AI",
  mistral: "Mistral",
  cohere: "Cohere",
  deepseek: "DeepSeek",
  "openai-codex": "Codex",
  twitter: "X (Twitter)",
  google: "Google",
  github: "GitHub",
  cloudflare: "Cloudflare",
  supabase: "Supabase",
  "supabase-management": "Supabase Management",
  railway: "Railway",
  facebook: "Facebook",
  instagram: "Instagram",
  linkedin: "LinkedIn",
  discord: "Discord",
  spotify: "Spotify",
  slack: "Slack",
  microsoft: "Microsoft",
  apple: "Apple",
  tiktok: "TikTok",
  twitch: "Twitch",
  reddit: "Reddit",
  telegram: "Telegram",
  "telegram-bot": "Telegram Bot",
  lark: "Lark",
  feishu: "Feishu",
};

export type ToolActivityStatus = "running" | "done" | "error" | "unknown" | "skipped";

/** [present participle, past, base] of a phrase, e.g. reading/read/read. */
type Forms = readonly [string, string, string];

const VERBS: Readonly<Record<string, readonly [string, string]>> = {
  get: ["Reading", "Read"],
  read: ["Reading", "Read"],
  fetch: ["Fetching", "Fetched"],
  download: ["Downloading", "Downloaded"],
  export: ["Exporting", "Exported"],
  list: ["Listing", "Listed"],
  search: ["Searching", "Searched"],
  query: ["Querying", "Queried"],
  find: ["Finding", "Found"],
  check: ["Checking", "Checked"],
  create: ["Creating", "Created"],
  add: ["Adding", "Added"],
  append: ["Appending", "Appended"],
  upload: ["Uploading", "Uploaded"],
  post: ["Posting", "Posted"],
  submit: ["Submitting", "Submitted"],
  send: ["Sending", "Sent"],
  reply: ["Replying to", "Replied to"],
  compose: ["Composing", "Composed"],
  forward: ["Forwarding", "Forwarded"],
  update: ["Updating", "Updated"],
  edit: ["Editing", "Edited"],
  patch: ["Updating", "Updated"],
  put: ["Updating", "Updated"],
  set: ["Setting", "Set"],
  rename: ["Renaming", "Renamed"],
  move: ["Moving", "Moved"],
  copy: ["Copying", "Copied"],
  delete: ["Deleting", "Deleted"],
  remove: ["Removing", "Removed"],
  clear: ["Clearing", "Cleared"],
  cancel: ["Cancelling", "Cancelled"],
  run: ["Running", "Ran"],
  execute: ["Running", "Ran"],
  invoke: ["Invoking", "Invoked"],
  call: ["Calling", "Called"],
  start: ["Starting", "Started"],
  stop: ["Stopping", "Stopped"],
  generate: ["Generating", "Generated"],
  connect: ["Connecting", "Connected"],
  grant: ["Granting", "Granted"],
  schedule: ["Scheduling", "Scheduled"],
  remember: ["Remembering", "Remembered"],
  forget: ["Forgetting", "Forgot"],
};

/** NyxID's own meta tools, phrased by what they do for the user. */
const META: Readonly<Record<string, Forms>> = {
  nyx__search_tools: [
    "Searching for tools",
    "Searched for tools",
    "search for tools",
  ],
  nyx__discover_services: [
    "Browsing the service catalog",
    "Browsed the service catalog",
    "browse the service catalog",
  ],
  nyx__list_connected_services: [
    "Checking connected services",
    "Checked connected services",
    "check connected services",
  ],
  nyx__connect_service: [
    "Preparing a connection link",
    "Prepared a connection link",
    "prepare a connection link",
  ],
  nyx__wait_for_connection: [
    "Waiting for the connection",
    "Waited for the connection",
    "finish the connection",
  ],
  nyx__call_tool: ["Calling a tool", "Called a tool", "call a tool"],
  nyx__ssh_exec: [
    "Running an SSH command",
    "Ran an SSH command",
    "run an SSH command",
  ],
  nyx__ssh_list_services: [
    "Listing SSH services",
    "Listed SSH services",
    "list SSH services",
  ],
  nyx__oracle_ask: ["Asking the Oracle", "Asked the Oracle", "ask the Oracle"],
  nyx__oracle_result: [
    "Checking the Oracle's answer",
    "Checked the Oracle's answer",
    "check the Oracle's answer",
  ],
};

/** Words that name the API surface rather than the thing acted on. */
const NOISE = new Set(["api", "v1", "v2", "v3"]);

function titleCase(words: readonly string[]): string {
  return words
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

/** "api-github-4" -> "GitHub"; "api-google-workspace-2" -> "Google Workspace". */
function serviceName(slug: string): string {
  const base = slug.replace(/^(api|llm|tools)-/, "").replace(/-\d+$/, "");
  return (
    (Object.hasOwn(PROVIDER_LABELS, base)
      ? PROVIDER_LABELS[base]
      : undefined) || titleCase(base.split("-").filter(Boolean))
  );
}

type ActivityMessage = { phrases: Forms; values: Record<string, string> };

/** Select complete English message keys first; translate exactly once at the edge. */
function operationForms(operation: string): ActivityMessage {
  const words = operation.toLowerCase().split(/[_\-.\s]+/)
    .filter((word) => word && !NOISE.has(word));
  const at = words.findIndex((word) => Object.hasOwn(VERBS, word));
  const verb = words[at];
  const pair = verb === undefined ? undefined : VERBS[verb];
  const object = words.filter((_, index) => index !== at).join(" ") || (pair ? "" : operation);
  const [running, done] = pair ?? ["Using", "Used"];
  const tail = object ? " {{object}}" : "";
  return {
    phrases: [running + tail, done + tail, "Couldn't " + (pair ? verb : "use") + tail],
    values: { object },
  };
}

function forms(label: string): ActivityMessage | null {
  const meta = Object.hasOwn(META, label) ? META[label] : undefined;
  if (meta) return { phrases: [meta[0], meta[1], "Couldn't " + meta[2]], values: {} };
  const split = label.indexOf("__");
  if (split <= 0 || split + 2 >= label.length) return null;
  const service = label.slice(0, split);
  const message = operationForms(label.slice(split + 2));
  if (service === "nyx" || service === "nyxid") return message;
  return {
    phrases: message.phrases.map((phrase) => phrase + " on {{service}}") as unknown as Forms,
    values: { ...message.values, service: serviceName(service) },
  };
}

/** Every state of the closed client vocabulary has a complete catalog template. */
export function toolActivityTranslationKeys(): string[] {
  const labels = Object.keys(VERBS).flatMap((verb) =>
    [`nyx__${verb}`, `nyx__${verb}_object`, `external__${verb}`, `external__${verb}_object`]);
  labels.push("nyx__unknown_object", "external__unknown_object", ...Object.keys(META));
  return [...new Set(labels.flatMap((label) => forms(label)!.phrases))];
}

/**
 * Human text for one tool call, derived from its identifier alone (activities
 * never carry arguments): "api-github-4__get_issue" -> "Read issue on GitHub".
 * Unrecognized identifiers are returned unchanged.
 */
export function describeToolActivity(
  label: string,
  status: ToolActivityStatus,
): string {
  const message = forms(label);
  if (!message || status === "unknown" || status === "skipped") return label;
  return translateApp(message.phrases[status === "running" ? 0 : status === "error" ? 2 : 1], message.values);
}
