import { translateApp } from "../../i18n/appTranslation";

// Each state owns a complete message, including punctuation and optional counts.
const threadLabels: Record<string, string> = {
  ":::": "Open chat {{title}} with {{agent}}",
  ":::unread": "Open chat {{title}} with {{agent}}, {{count}} unread",
  ":current::": "Open chat {{title}} with {{agent}}, current thread",
  ":current::unread": "Open chat {{title}} with {{agent}}, current thread, {{count}} unread",
  "Running:::": "Open chat {{title}} with {{agent}}, Running",
  "Running:::unread": "Open chat {{title}} with {{agent}}, Running, {{count}} unread",
  "Running:current::": "Open chat {{title}} with {{agent}}, Running, current thread",
  "Running:current::unread": "Open chat {{title}} with {{agent}}, Running, current thread, {{count}} unread",
  "Needs you:::": "Open chat {{title}} with {{agent}}, Needs you",
  "Needs you:::unread": "Open chat {{title}} with {{agent}}, Needs you, {{count}} unread",
  "Needs you:current::": "Open chat {{title}} with {{agent}}, Needs you, current thread",
  "Needs you:current::unread": "Open chat {{title}} with {{agent}}, Needs you, current thread, {{count}} unread",
};

const crewLabels: Record<string, string> = {
  ":::": "Open crew {{name}}",
  ":::unread": "Open crew {{name}}, {{count}} unread",
  ":current::": "Open crew {{name}}, current crew",
  ":current::unread": "Open crew {{name}}, current crew, {{count}} unread",
  "Running:::": "Open crew {{name}}, {{working}} running",
  "Running:::unread": "Open crew {{name}}, {{working}} running, {{count}} unread",
  "Running:current::": "Open crew {{name}}, {{working}} running, current crew",
  "Running:current::unread": "Open crew {{name}}, {{working}} running, current crew, {{count}} unread",
};

const disclosureLabels: Record<string, string> = {
  "Collapse:::": "Collapse chats for {{name}}",
  "Collapse:::unread": "Collapse chats for {{name}}, {{count}} unread",
  "Collapse::detail:": "Collapse chats for {{name}}, {{details}}",
  "Collapse::detail:unread": "Collapse chats for {{name}}, {{details}}, {{count}} unread",
  "Expand:::": "Expand chats for {{name}}",
  "Expand:::unread": "Expand chats for {{name}}, {{count}} unread",
  "Expand::detail:": "Expand chats for {{name}}, {{details}}",
  "Expand::detail:unread": "Expand chats for {{name}}, {{details}}, {{count}} unread",
};

const recentLabels: Record<string, string> = {
  ":::": "Open recent {{title}}",
  ":::unread": "Open recent {{title}}, {{count}} unread",
  "Running:::": "Open recent {{title}}, Running",
  "Running:::unread": "Open recent {{title}}, Running, {{count}} unread",
  "Needs you:::": "Open recent {{title}}, Needs you",
  "Needs you:::unread": "Open recent {{title}}, Needs you, {{count}} unread",
};

export function workspaceThreadLabel(title: string, agent: string, status: string | undefined, current: boolean, count = 0) {
  const key = [status ?? "", current ? "current" : "", "", count > 0 ? "unread" : ""].join(":");
  return translateApp(threadLabels[key]!, { title, agent, count });
}
export function workspaceCrewLabel(name: string, working: number, current: boolean, count = 0) {
  const key = [working ? "Running" : "", current ? "current" : "", "", count > 0 ? "unread" : ""].join(":");
  return translateApp(crewLabels[key]!, { name, working, count });
}
export function workspaceDisclosureLabel(name: string, expanded: boolean, details?: string, count = 0) {
  const key = [expanded ? "Collapse" : "Expand", "", details ? "detail" : "", count > 0 ? "unread" : ""].join(":");
  return translateApp(disclosureLabels[key]!, { name, details: details ?? "", count });
}
export function workspaceRecentLabel(title: string, status?: string, count = 0) {
  const key = [status ?? "", "", "", count > 0 ? "unread" : ""].join(":");
  return translateApp(recentLabels[key]!, { title, count });
}
