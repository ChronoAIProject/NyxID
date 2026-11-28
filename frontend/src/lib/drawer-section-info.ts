import { createContext } from "react";
import { appI18n } from "@/i18n/instance";
import { translateUi } from "@/i18n/use-ui-translation";

/** Explicit drawer copy; dynamic service, chat and skill headings supply their own context. */
export const DRAWER_SECTION_INFO: Readonly<Record<string, string>> = {
  Overview: "Identity, working instructions, tone, ownership and lifecycle settings for this agent. Search results open the matching setting.",
  Channels: "Messaging bots and chats connected to this agent. Search results open the matching connection or chat.",
  Profile: "Identity, purpose and working instructions. Tone and style shapes communication; Memory keeps facts and Skills provide reusable procedures.",
  Instructions: "Define the agent’s purpose and working behavior. These instructions do not grant permissions. Memory retains facts and preferences; Skills provide reusable procedures.",
  "Tone and style": "Set communication style and personality without changing permissions.",
  "Call instructions": "Voice and phone guidance for new calls using this agent’s existing permissions. When available, optional Device voice setup is in Access.",
  "Approval behavior": "Choose when the agent asks before acting within existing permissions. Inherit the workspace setting or set an agent override; protected organization connections can still require your instruction or approval.",
  "Channel progress reply": "An agent-wide short reply while external checks take time, followed by the completed answer in the same chat. This applies across the agent’s channels.",
  "Device voice": "Optional device integration. Create a voice connection, then check the device Agent Key’s existing service policy. All-services Keys may already permit it; creation alone does not prove device readiness.",
  "Access grants": "Services and account permissions granted to this agent. Approval behavior controls when it asks before acting within those limits.",
  "Shared profile": "The shared agent’s display name, description and personality. Changes apply to everyone who uses it.",
  Ownership: "Who owns this agent and who may maintain it. Organization roles and sharing roles determine the actions available to you.",
  Sharing: "Choose who can use this agent and which of its granted services they may use. Sharing never gives others your credentials.",
  "People and organizations": "Choose the people and organizations that may use this shared agent.",
  "Services for shared users": "Limit which granted services shared users may call through this agent.",
  Automations: "Scheduled and event-driven tasks that run for this agent. Open Automations to manage their timing and delivery.",
  "Danger zone": "Destroying stops this specialist. Deleting removes its record; each action requires confirmation.",
  Lifecycle: "Destroying stops this specialist. Deleting removes its record; each action requires confirmation.",
  Access: "Service and machine permissions determine what this agent can use. Approval behavior controls when it asks before acting within those permissions.",
  Services: "Choose which connected services this specialist may call. Operation and guest permissions can narrow that access further.",
  Account: "Allow this specialist to read account information. This does not grant additional service or machine access.",
  Machines: "Choose the machines this specialist may use. Each machine’s capability and workspace settings still apply.",
  "Saved logins": "Choose saved logins this specialist may use on its granted machines. Machine confirmation and login restrictions still apply.",
  "What others in its chats may do": "Limit what visitors in this agent’s chats may do through each granted service. These limits cannot exceed the agent’s own access.",
  "Service operations": "Narrow a service grant to selected operations or exact method and path rules. Each service saves against its own revision.",
  "Guest service operations": "Choose which operations visitors may use within the agent’s service permissions. These limits never broaden the agent’s grant.",
  "Select operations": "Select the published operations this agent may call. An empty selection blocks every operation.",
  "Custom operations": "Define exact HTTP methods and paths for services without published operations. Rules narrow the service grant.",
  "Machine access": "Choose a machine to review its workspace, browser and allowed capabilities. Access remains subject to the machine’s current support and permissions.",
  "Workspace and browser": "Choose how this agent’s execution is separated from other users on this machine. Required separation blocks execution when unavailable.",
  "Allowed capabilities": "Allow individual tools on this machine, such as commands, files or browser control. The machine must also support each capability.",
  Connections: "The connected accounts available to this agent. Account order decides which connection is tried first for a service.",
  "Connection order": "Arrange the accounts used for a service. Personal accounts are preferred by default.",
  "Connection consent": "Review requests to use your own connection through a shared agent. You may allow once, always, or skip.",
  "Attached skills": "Skills give this agent reusable instructions. Attached versions remain pinned until you choose a different version.",
  Skills: "Reusable instructions attached to this agent. Its pinned versions determine which instructions it receives.",
  "Pinned skill": "Review the version attached to this agent. Changing or disabling it updates the agent’s skill configuration.",
  "Find skills": "Search the skill catalog and choose a version to attach. This agent can have up to sixteen attached skills.",
  "Choose version": "Choose the published skill version to attach. Review it before confirming the change.",
  "Skill history": "Changes to this agent’s attached skills, shown newest first.",
  Activity: "Changes to this agent’s attached skills, shown newest first.",
  Draft: "Edit a private copy of this skill. The attached version stays active until the draft is published and attached.",
  "Preload settings": "Include selected skill files in every owner chat turn. Guest chats do not receive preloaded files.",
  "Learning proposals": "Analysis of eligible conversations may produce a private proposal or none. Requesting publication can publish and attach immediately in the approving chat’s Full access mode; otherwise a confirmation card is returned.",
  Proposal: "Review the suggested skill, its evidence and publication status before deciding what to do.",
  Memory: "Notes this agent can remember across chats. Shared and organization memory follows the permissions shown here.",
  Notes: "Notes this agent can remember across chats. The ownership and sharing rules shown here determine who can change them.",
  "Add a note": "Write something this agent should remember across chats. Saving adds it to the agent’s memory.",
  "Edit note": "Update this remembered note. The change is saved only after you confirm it.",
  "Your private notes": "Notes kept private to you and this agent in your private chats. Other users cannot read them.",
  "Published team memory": "Notes explicitly published by the owner or a manager for everyone who can use this agent.",
  "Private notes": "Notes kept private to your use of the shared agent.",
  "Shared notes": "Notes available to everyone who uses this shared agent, according to their role.",
  "Connected bot": "The messaging bot connected to this agent. Choose who answers and manage which chats and people may use it.",
  "Channel bots": "Connect an available messaging bot to this agent. Existing bot and account verification rules still apply.",
  "Available bots": "Choose one of your available messaging bots to connect to this agent.",
  "Connect another bot": "Choose an available messaging bot and connect it to this agent.",
  "Connection status": "The current state of this agent’s messaging connections. Errors here may prevent messages from reaching it.",
  "Who can talk to this bot": "Choose the bot’s private-chat audience or organization guest access. Personal guests use the owner’s billing; organization guests follow the displayed service and wallet prerequisites.",
  "Chats and posting": "Manage posting and the bot’s behavior in each connected chat. Bot-wide guest access is in Who can talk to this bot.",
  "Private chats": "Allow posts in new private chats, or explicitly apply the setting to existing chats. This does not change who can talk to the bot.",
  Threads: "Choose which conversation threads this bot follows. Thread replies still use this chat’s reply and participant settings.",
  "Owner link": "Verify your messaging account as the bot owner. Complete verification using the link shown here.",
  "Guest replies": "Review requests to reply to other people in this agent’s chats. Service permissions still apply.",
};
export const DrawerSectionInfoContext = createContext(false);
let localizedTitles: Map<string, string> | undefined;
const invalidateTitles = () => { localizedTitles = undefined; };
appI18n.on("languageChanged loaded", invalidateTitles);
appI18n.store.on("added", invalidateTitles);
appI18n.store.on("removed", invalidateTitles);
export function drawerSectionInfo(title: string): string | undefined {
  // Group headings already carry their localized display title. Resolve that
  // title for help only; operational IDs and search targets stay unchanged.
  if (Object.hasOwn(DRAWER_SECTION_INFO, title)) return translateUi(DRAWER_SECTION_INFO[title]!);
  localizedTitles ??= new Map(Object.keys(DRAWER_SECTION_INFO).map((key) => [translateUi(key === "Memory" ? "Agent memory" : key), key]));
  const key = localizedTitles.get(title);
  const matches = (template: string) => {
    const [before, after] = translateUi(template, { value1: "__value__" }).split("__value__");
    return after !== undefined && title.startsWith(before!) && title.endsWith(after);
  };
  const help = (key ? DRAWER_SECTION_INFO[key] : undefined) ?? (matches("Rule {{value1}}") ? "An exact method and path allowed by this operation scope. Removing a rule removes that permission." : matches("Preloaded · {{value1}} bytes") ? DRAWER_SECTION_INFO["Preload settings"] : undefined);
  return help ? translateUi(help) : undefined;
}
