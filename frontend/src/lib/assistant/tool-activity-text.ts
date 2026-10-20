import { getProviderBrand } from "@/lib/provider-branding";

export type ToolActivityStatus = "running" | "done" | "error";

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
  nyx__search_tools: ["Searching for tools", "Searched for tools", "search for tools"],
  nyx__discover_services: ["Browsing the service catalog", "Browsed the service catalog", "browse the service catalog"],
  nyx__list_connected_services: ["Checking connected services", "Checked connected services", "check connected services"],
  nyx__connect_service: ["Preparing a connection link", "Prepared a connection link", "prepare a connection link"],
  nyx__wait_for_connection: ["Waiting for the connection", "Waited for the connection", "finish the connection"],
  nyx__call_tool: ["Calling a tool", "Called a tool", "call a tool"],
  nyx__ssh_exec: ["Running an SSH command", "Ran an SSH command", "run an SSH command"],
  nyx__ssh_list_services: ["Listing SSH services", "Listed SSH services", "list SSH services"],
  nyx__oracle_ask: ["Asking the Oracle", "Asked the Oracle", "ask the Oracle"],
  nyx__oracle_result: ["Checking the Oracle's answer", "Checked the Oracle's answer", "check the Oracle's answer"],
};

/** Words that name the API surface rather than the thing acted on. */
const NOISE = new Set(["api", "v1", "v2", "v3"]);

function titleCase(words: readonly string[]): string {
  return words.map((word) => word.charAt(0).toUpperCase() + word.slice(1)).join(" ");
}

/** "api-github-4" -> "GitHub"; "api-google-workspace-2" -> "Google Workspace". */
function serviceName(slug: string): string {
  const base = slug.replace(/^(api|llm|tools)-/, "").replace(/-\d+$/, "");
  return getProviderBrand(base).label || titleCase(base.split("-").filter(Boolean));
}

/** "get_file_contents" -> reading/read/get "file contents". */
function operationForms(operation: string): Forms {
  const words = operation.toLowerCase().split(/[_\-.\s]+/).filter((word) => word && !NOISE.has(word));
  const at = words.findIndex((word) => word in VERBS);
  const verb = words[at];
  const verbForms = verb === undefined ? undefined : VERBS[verb];
  if (verb === undefined || !verbForms) {
    const object = words.join(" ") || operation;
    return [`Using ${object}`, `Used ${object}`, `use ${object}`];
  }
  const [ing, past] = verbForms;
  const object = words.filter((_, index) => index !== at).join(" ");
  const tail = object ? ` ${object}` : "";
  return [`${ing}${tail}`, `${past}${tail}`, `${verb}${tail}`];
}

function forms(label: string): Forms | null {
  const meta = META[label];
  if (meta) return meta;
  const split = label.indexOf("__");
  if (split <= 0 || split + 2 >= label.length) return null;
  const service = label.slice(0, split);
  const [ing, past, base] = operationForms(label.slice(split + 2));
  if (service === "nyx" || service === "nyxid") return [ing, past, base];
  const where = ` on ${serviceName(service)}`;
  return [`${ing}${where}`, `${past}${where}`, `${base}${where}`];
}

/**
 * Human text for one tool call, derived from its identifier alone (activities
 * never carry arguments): "api-github-4__get_issue" -> "Read issue on GitHub".
 * Unrecognized identifiers are returned unchanged.
 */
export function describeToolActivity(label: string, status: ToolActivityStatus): string {
  const phrase = forms(label);
  if (!phrase) return label;
  if (status === "running") return phrase[0];
  if (status === "error") return `Couldn't ${phrase[2]}`;
  return phrase[1];
}
