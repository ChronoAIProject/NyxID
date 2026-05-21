import type {
  NodeAdminInfo,
  NodeInfo,
  NodePendingCredentialInjectionMethod,
} from "@/types/nodes";
import type { KeyInfo } from "@/types/keys";

export function nodeOwnerLabel(
  owner: NodeInfo["owner"],
  currentUserId: string | null,
): string {
  if (owner.kind === "user" && owner.id === currentUserId) {
    return "You";
  }
  return owner.display_name;
}

export function adminDisplayName(
  admin: NodeAdminInfo,
  currentUserId: string | null,
) {
  if (admin.user_id === currentUserId) {
    return "You";
  }
  return admin.display_name ?? admin.email ?? admin.user_id;
}

export function canManageNode(
  node: NodeInfo | undefined,
  currentUserId: string | null,
  admins: readonly NodeAdminInfo[] | undefined,
): boolean {
  if (!node || !currentUserId) {
    return false;
  }
  if (node.owner.kind === "user") {
    return node.owner.id === currentUserId;
  }
  return (admins ?? []).some((admin) => admin.user_id === currentUserId);
}

export function keyOwnerId(
  key: KeyInfo,
  currentUserId: string | null,
): string | null {
  const source = key.credential_source;
  if (!source || source.type === "personal") {
    return currentUserId;
  }
  return source.org_id;
}

export function injectionMethodLabel(
  method: NodePendingCredentialInjectionMethod,
): string {
  switch (method) {
    case "query-param":
      return "Query param";
    case "path-prefix":
      return "Path prefix";
    case "header":
      return "Header";
  }
}

export function defaultFieldNameForMethod(
  method: NodePendingCredentialInjectionMethod,
): string {
  switch (method) {
    case "query-param":
      return "api_key";
    case "path-prefix":
      return "api";
    case "header":
      return "X-API-Key";
  }
}
