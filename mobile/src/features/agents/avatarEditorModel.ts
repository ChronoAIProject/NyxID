import {
  avatarPatch,
  defaultAvatar,
  readableAvatar,
  resolveAvatar,
  sameAvatar,
  type AvatarConfig,
} from "../../lib/avatars/agentAvatars";
import type { AssistantAgent } from "./agentContracts";

/** What the editor draws and saves for `agent` with `draft` chosen. */
export function avatarEditorState(agent: AssistantAgent, draft: AvatarConfig) {
  const fallback = defaultAvatar(agent.id, agent.kind);
  const current = resolveAvatar({
    id: agent.id,
    kind: agent.kind,
    avatar: agent.avatar ?? null,
  });
  const stored = Boolean(agent.avatar);
  const dirty = !sameAvatar(draft, current);
  // Back to default clears the stored choice rather than storing the default.
  const clearing = stored && sameAvatar(draft, fallback);
  return {
    fallback,
    current,
    dirty,
    atDefault: sameAvatar(draft, fallback),
    patch: avatarPatch(clearing ? null : draft),
  };
}

/** The stored avatar changed since the editor opened (another device or
 * person); saving would overwrite it. */
export function avatarChangedElsewhere(
  baseline: AssistantAgent,
  latest: AssistantAgent,
): boolean {
  const before = readableAvatar(baseline.avatar) ? baseline.avatar : null;
  const now = readableAvatar(latest.avatar) ? latest.avatar : null;
  if (!before || !now)
    return (
      JSON.stringify(baseline.avatar ?? null) !==
      JSON.stringify(latest.avatar ?? null)
    );
  return !sameAvatar(before, now);
}

export { EDITOR_SLOTS as AVATAR_EDITOR_SLOTS } from "../../../../shared/agent-avatars/editor";
import { EDITOR_SLOTS } from "../../../../shared/agent-avatars/editor";

/** Every phrase the editor shows; each is in both mobile catalogs. */
export const AVATAR_EDITOR_COPY = [
  "Avatar",
  "Avatar preview",
  "Preview expression",
  "Randomise",
  "Reset to default",
  "Changes apply everywhere this agent appears.",
  "How this agent looks in chats, groups and lists. Its expression follows what it is doing; the status text stays as it is.",
  "Could not save the avatar. Try again.",
  "This agent's avatar can no longer be changed here.",
  "This avatar changed elsewhere. Go back and review it before saving.",
  "You're offline. Connect to save changes.",
  "Discard changes?",
  "Your unsaved avatar change will be lost.",
  "Loading agent",
  "Choose every avatar part from the list",
  "Edit avatar",
  ...EDITOR_SLOTS.map((slot) => slot.label),
] as const;
