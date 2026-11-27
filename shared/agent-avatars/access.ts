/**
 * Who may restyle an agent and the PATCH body that does it. Shared by web
 * and mobile so both offer the editor to exactly the same people; the
 * server still decides (profile authority on `PATCH /agents/{id}`).
 */
import { type AvatarConfig } from "./model";

/** The agent fields the decision reads (web and mobile DTOs both carry them). */
export interface AvatarEditorAgent {
  readonly status: string;
  readonly share_role?: string | null;
  readonly can_maintain?: boolean;
}

/** Maintainers (owners, org admins and members who maintain) and shared
 * managers; never a destroyed agent. */
export function canEditAvatar(agent: AvatarEditorAgent): boolean {
  if (agent.status === "destroyed") return false;
  if (agent.share_role) return agent.share_role === "manager";
  return agent.can_maintain !== false;
}

/** PATCH body: an object sets the avatar, null clears it (back to default). */
export function avatarPatch(avatar: AvatarConfig | null): {
  avatar: AvatarConfig | null;
} {
  return {
    avatar: avatar
      ? { template: avatar.template, choices: { ...avatar.choices } }
      : null,
  };
}
