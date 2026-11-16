import { sectionNavigation } from "@/lib/section-navigation";

export const AGENT_DETAILS_SECTIONS = [
  "overview",
  "overview/persona",
  "overview/access",
  "overview/chat",
  "overview/voice",
  "overview/automations",
  "overview/ownership",
  "grants",
  "grants/services",
  "grants/operations",
  "grants/guests",
  "grants/machines",
  "grants/connections",
  "grants/voice",
  "skills",
  "skills/installed",
  "skills/learning",
  "memory",
  "channels",
] as const;
export type AgentDetailsSection = (typeof AGENT_DETAILS_SECTIONS)[number];
export const agentDetailsSections = sectionNavigation(AGENT_DETAILS_SECTIONS);
export const AGENT_DETAILS_TITLES: Record<AgentDetailsSection, string> = {
  overview: "Overview",
  "overview/persona": "Name & persona",
  "overview/access": "Access mode",
  "overview/chat": "Chat behaviour",
  "overview/voice": "Voice",
  "overview/automations": "Automations",
  "overview/ownership": "Ownership",
  grants: "Access",
  "grants/services": "Connected services",
  "grants/operations": "Operation limits",
  "grants/guests": "Guest access",
  "grants/machines": "Machines",
  "grants/connections": "Connections",
  "grants/voice": "Voice service",
  skills: "Skills",
  "skills/installed": "Installed skills",
  "skills/learning": "Learning proposals",
  memory: "Agent memory",
  channels: "Channels",
};
export const AGENT_PROFILE_FIELDS = {
  "overview/persona": {
    name: "Name",
    display_name: "Display name",
    description: "Role",
    persona: "Persona",
    model: "Model",
  },
  "overview/access": { access_mode_override: "Access mode" },
  "overview/chat": { channel_checking_notice: "Channel progress reply" },
  "overview/voice": { voice_instructions: "Voice instructions" },
} as const;

export const AGENT_NOTES_LABEL = "Notes";
export const AGENT_DETAILS_LABELS: Partial<Record<AgentDetailsSection, readonly string[]>> = {
  "grants/services": ["Services", "Read my account", "Connected services and account"],
  "grants/machines": ["Machines", "Saved logins", "Machine capabilities"],
  "grants/operations": ["Operation limits"],
  "grants/guests": ["Guest access", "Guest operation limits"],
  "grants/connections": ["Connections", "Manage connected accounts"],
  "grants/voice": ["Device voice"],
  "skills/installed": ["Installed skills", "Skill history"],
  "skills/learning": ["Automatic learning", "Threshold", "Learning proposals"],
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
    "overview/access": owned,
    "overview/chat": owned,
    "overview/voice": owned,
    "overview/automations": owned && agent.can_use !== false,
    "grants/operations": owned && agent.kind === "specialist" && maintained,
    "grants/guests":
      owned &&
      agent.kind === "specialist" &&
      maintained &&
      (agent.owner_kind !== "org" || (options.orgChannels && agent.org_role === "admin")),
    "grants/machines": owned,
    "grants/connections": owned,
    "grants/voice":
      owned &&
      options.voice &&
      Boolean(options.owner) &&
      agent.owner_kind !== "org" &&
      agent.status !== "destroyed",
    "skills/learning": owned && options.learning,
    channels:
      owned &&
      (agent.owner_kind !== "org" || options.orgChannels || Boolean(options.hasOrgChannel)) &&
      agent.status !== "destroyed",
    memory: Boolean(agent.share_role) || maintained,
  };
}
