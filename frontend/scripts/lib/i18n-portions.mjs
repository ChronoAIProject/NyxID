/**
 * Ordered ownership map for app catalog usage. The first matching pattern wins.
 * Keep this list stable: it is also used by the migration/adoption tool when a
 * pre-split branch adds a new key.
 */
export const PORTIONS = [
  { name: "assistant-chat", patterns: [/^src\/components\/assistant\/chat(?:\/|[-.])/, /^src\/components\/assistant\/.*(?:chat|composer|message|attachment)/i, /^src\/lib\/assistant\/(?:chat|message|attachment|title|turn|interrupted)/] },
  { name: "assistant-agents", patterns: [/^src\/components\/assistant\/.*(?:agent|skill|group)/i, /^src\/pages\/assistant-cma-(?:agent|sandboxes)/, /^src\/lib\/assistant\/(?:agent|skill|plugins|capacity)/] },
  { name: "assistant-account", patterns: [/^src\/components\/assistant\/.*(?:account|settings|profile|workspace)/i, /^src\/lib\/assistant\/account/ ] },
  { name: "assistant-approvals", patterns: [/^src\/components\/assistant\/.*(?:approval|decision|action)/i, /^src\/lib\/assistant\/(?:approval|action)/, /^src\/pages\/approval/ ] },
  { name: "automations", patterns: [/^src\/pages\/automations/, /^src\/components\/assistant\/.*automation/i, /^src\/hooks\/use-automation/, /^src\/lib\/automation/ ] },
  { name: "assistant", patterns: [/^src\/components\/assistant\//, /^src\/lib\/assistant\//, /^src\/pages\/assistant(?:\.tsx|\/)/] },
  { name: "admin", patterns: [/^src\/pages\/admin(?:-|\/)/, /^src\/components\/admin(?:-|\/)/, /^src\/hooks\/use-admin/, /^src\/lib\/admin/ ] },
  { name: "cli-pair", patterns: [/^src\/pages\/cli-pair\//, /^src\/components\/cli-wizard\//, /^src\/wizard-entry\.tsx$/, /^src\/hooks\/use-cli/ ] },
  { name: "auth", patterns: [/^src\/pages\/(?:login|register|oauth|consents|connect-link|cli-auth)/, /^src\/components\/auth(?:\/|[-.])/, /^src\/hooks\/use-(?:auth|oauth|login)/, /^src\/lib\/(?:auth|oauth)/ ] },
  { name: "billing", patterns: [/^src\/pages\/billing/, /^src\/components\/billing(?:-|\/)/, /^src\/hooks\/use-(?:billing|usage)/, /^src\/lib\/billing/ ] },
  { name: "channels", patterns: [/^src\/pages\/channel/, /^src\/pages\/telegram/, /^src\/components\/channels\//, /^src\/components\/telegram/, /^src\/hooks\/use-channel/, /^src\/lib\/channel/ ] },
  { name: "machines", patterns: [/^src\/pages\/(?:machine|machines)/, /^src\/components\/machines\//, /^src\/hooks\/use-machine/, /^src\/lib\/machine/ ] },
  { name: "platform", patterns: [/^src\/pages\/platform/, /^src\/components\/platform/, /^src\/hooks\/use-platform/, /^src\/lib\/platform/ ] },
  { name: "services", patterns: [/^src\/pages\/(?:service|services|provider|providers|developer-apps)/, /^src\/components\/(?:services|providers|service-accounts|developer-apps)\//, /^src\/hooks\/use-(?:service|provider|catalog)/, /^src\/lib\/(?:service|provider|catalog)/ ] },
  { name: "settings", patterns: [/^src\/pages\/settings/, /^src\/components\/settings\//, /^src\/pages\/notification-settings/, /^src\/hooks\/use-settings/, /^src\/lib\/settings/ ] },
  { name: "credentials", patterns: [/^src\/pages\/(?:keys|key-detail|api-key|ai-setup)/, /^src\/components\/dashboard\/(?:api-key|.*(?:key|credential))/, /^src\/hooks\/use-(?:keys|api-key|agent-binding)/ ] },
  { name: "organizations", patterns: [/^src\/pages\/org/, /^src\/components\/orgs\//, /^src\/components\/dashboard\/.*org/, /^src\/hooks\/use-org/ ] },
  { name: "nodes", patterns: [/^src\/pages\/(?:node|nodes|devices|ssh-terminal)/, /^src\/components\/dashboard\/.*(?:node|device|ssh)/, /^src\/hooks\/use-(?:node|device|ssh)/ ] },
  { name: "dashboard", patterns: [/^src\/pages\/(?:dashboard|tools|triggers|saved-logins|design-system|integration-guide)/, /^src\/components\/(?:dashboard|navigation|layout|connect-link)\//, /^src\/hooks\/use-(?:tool|trigger)/ ] },
  { name: "docs", patterns: [/^src\/features\/docs\//, /^src\/pages\/docs/ ] },
  { name: "shared", patterns: [/^src\//] },
  { name: "unattributed", patterns: [] },
];

export function portionForPath(file) {
  return PORTIONS.find((portion) => portion.patterns.some((pattern) => pattern.test(file)))?.name ?? "shared";
}

export function portionNames() {
  return PORTIONS.map(({ name }) => name);
}
