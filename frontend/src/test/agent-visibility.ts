import { createRequire } from "node:module";

// This dependency's ESM export omits its declarations; its CommonJS entry exposes the same API.
const { computeAccessibleName, getRole } = createRequire(import.meta.url)("dom-accessibility-api") as {
  computeAccessibleName: (element: Element) => string;
  getRole: (element: Element) => string | null;
};

export const agentVisibilityCases: Record<string, Record<string, unknown>> = {
  nyxbot: { kind: "nyxbot", name: "nyxbot" },
  personal: {},
  orgAdmin: { owner_kind: "org", owner_id: "org", owner_name: "Team", org_role: "admin", can_maintain: true },
  orgMaintainer: { owner_kind: "org", owner_id: "org", owner_name: "Team", org_role: "member", can_maintain: true },
  orgReader: { owner_kind: "org", owner_id: "org", owner_name: "Team", org_role: "viewer", can_maintain: false },
  sharedRecipient: { share_role: "user", owner_id: "other", owner_name: "Other", can_maintain: false },
  sharedManager: { share_role: "manager", owner_id: "other", owner_name: "Other", can_maintain: true },
  destroyed: { status: "destroyed", destroyed_at: "2026-09-28T00:00:00Z" },
};

/** Ignore category/back/help controls: their labels intentionally follow the new grouping. */
export function agentControls() {
  return [...document.querySelectorAll<HTMLElement>('input,textarea,button,[role="switch"],[role="checkbox"],[role="combobox"],[role="radiogroup"],a')].flatMap(el => {
    if (el.closest('[hidden],[inert],[aria-hidden="true"]') || getComputedStyle(el).display === "none") return [];
    const role = getRole(el);
    const label = computeAccessibleName(el);
    const page = el.closest("[data-grouped-detail]");
    if (!label || role === "tab" || role === "radio" || /^About |^Search .* agent details$|^Close agent details$/.test(label) || el.hasAttribute("data-drawer-page-trigger") || page?.querySelector("button") === el) return [];
    // The checkmark radio list replaces the old single select; authority and options are asserted separately.
    if (role === "radiogroup") return [{ role: "combobox", label, disabled: Boolean(el.querySelector(":disabled")), readOnly: false }];
    return [{ role, label, disabled: el.matches(':disabled,[aria-disabled="true"]'), readOnly: el.hasAttribute("readonly") }];
  });
}
export function sortedAgentControls(controls: ReturnType<typeof agentControls>) {
  return [...new Map(controls.map(control => [`${control.role}: ${control.label}`, control])).values()].sort((a,b) => `${a.role}: ${a.label}`.localeCompare(`${b.role}: ${b.label}`));
}

export const agentVisibilityFeatures = ["assistant:model-controls", "assistant:voice-agent-key", "assistant:agent-learning", "assistant:operation-scopes", "assistant:guest-operation-scopes", "assistant:shared-agents", "nyxbot:org-specialist-channels"];
export function agentVisibilityResponse(endpoint: string): unknown | undefined {
  if (/\/(private-notes|shared-memory)$/.test(endpoint)) return { revision: 2, notes: [{ id: "shared-note", text: "Team convention" }] };
  if (endpoint.endsWith("/consents")) return { consents: [] };
  if (endpoint.endsWith("/voice-service")) return { service: null };
  if (endpoint.endsWith("/learning")) return { enabled: true, threshold: 15 };
  if (endpoint.endsWith("/learning/proposals")) return { proposals: [] };
  if (endpoint.endsWith("/operations") || endpoint.endsWith("/guest-operations")) return [];
  if (endpoint.endsWith("/group-access")) return { groups: [] };
  return undefined;
}

export function agentReadOnlyRows() {
  return [...document.querySelectorAll<HTMLElement>("[data-grouped-row]")].filter(el =>
    !el.closest('[hidden],[inert],[aria-hidden="true"]') &&
    !el.matches('button,a,[role="radio"]') &&
    !el.querySelector('input,textarea,button,a,[role="switch"],[role="checkbox"],[role="combobox"]'),
  ).map(el => el.textContent!.replace(/\s+/g, " ").trim()).filter(Boolean);
}
