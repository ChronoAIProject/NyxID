import { sectionNavigation } from "@/lib/section-navigation";

export const AGENT_DETAILS_SECTIONS = [
  "overview",
  "overview/persona",
  "overview/voice",
  "overview/automations",
  "grants",
  "grants/services",
  "grants/machines",
  "grants/connections",
  "skills",
  "memory",
  "channels",
] as const;
export type AgentDetailsSection = (typeof AGENT_DETAILS_SECTIONS)[number];
export const agentDetailsSections = sectionNavigation(AGENT_DETAILS_SECTIONS);
export const AGENT_DETAILS_TITLES: Record<AgentDetailsSection, string> = {
  overview: "Overview",
  "overview/persona": "Persona & notes",
  "overview/voice": "Voice",
  "overview/automations": "Automations",
  grants: "Access",
  "grants/services": "Connected services",
  "grants/machines": "Machines",
  "grants/connections": "Connections",
  skills: "Skills",
  memory: "Agent memory",
  channels: "Channels",
};
export const AGENT_PROFILE_FIELDS = {
  overview: {
    name: "Name",
    display_name: "Display name",
    description: "Role",
    model: "Model",
    access_mode_override: "Access mode",
    channel_checking_notice: "Channel progress reply",
  },
  "overview/persona": { persona: "Persona", description: "Role" },
  "overview/voice": { voice_instructions: "Voice instructions" },
} as const;

export const AGENT_NOTES_LABEL = "Notes";
export const AGENT_DETAILS_LABELS: Partial<Record<AgentDetailsSection, readonly string[]>> = {
  "grants/services": ["Services", "Read my account", "Connected services and account", "Operation limits", "Guest access", "Guest operation limits"],
  "grants/machines": ["Machines", "Saved logins", "Machine capabilities"],
  "grants/connections": ["Connections", "Manage connected accounts"],
  grants: ["Device voice"],
  skills: ["Installed skills", "Skill history", "Automatic learning", "Threshold", "Learning proposals"],
  memory: ["Remember", "Forget", "Agent memory"],
  channels: ["Connect another bot", "Channels"],
};

/** Match the underlying editors' authority and feature gates, including legacy channels. */
export function agentDetailsAvailability(
  agent: import("@/schemas/assistant-nyxagent").AssistantAgent,
  options: {
    orgChannels: boolean;
    hasOrgChannel?: boolean;
    voice: boolean;
    learning: boolean;
    owner?: string;
  },
): Partial<Record<AgentDetailsSection, boolean>> {
  const owned = !agent.share_role;
  const maintained = agent.can_maintain !== false;
  return {
    "overview/persona": owned,
    "overview/voice": owned,
    "overview/automations": owned && agent.can_use !== false,
    "grants/machines": owned,
    "grants/connections": owned,
    channels:
      owned &&
      (agent.owner_kind !== "org" || options.orgChannels || Boolean(options.hasOrgChannel)) &&
      agent.status !== "destroyed",
    memory: Boolean(agent.share_role) || maintained,
  };
}
