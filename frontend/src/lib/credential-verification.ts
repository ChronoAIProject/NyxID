import type { KeyInfo } from "@/types/keys";
import { getStatusMeta, type StatusMeta } from "./status-contract";

type Connection = Pick<KeyInfo, "credential_type" | "status" | "credential_binding" | "credential_verification" | "node_id" | "has_node_binding" | "credential_missing">;

/** Missing evidence from an older server must never imply provider acceptance. */
export function credentialVerificationMeta(key: Connection): StatusMeta | undefined {
  if (!isSavedCredential(key)) return undefined;
  if (key.status !== "active") return getStatusMeta("credential", key.status);
  if (key.credential_missing) return getStatusMeta("user_service_credential", "unavailable");
  const state = key.credential_verification?.state;
  const reason = key.credential_verification?.reason_code;
  const name = state === "verified" ? "verified" : state === "rejected"
    ? (reason === "provider_forbidden" ? "forbidden" : "rejected") : "not_verified";
  return getStatusMeta("credential_verification", name);
}

export function isSavedCredential(key: Pick<Connection, "credential_type" | "credential_binding" | "node_id" | "has_node_binding">): boolean {
  return ["api_key", "bearer"].includes(key.credential_type) && key.credential_binding !== "platform"
    && !key.node_id && !key.has_node_binding;
}

export function connectionVerificationLabel(keys: readonly KeyInfo[]): string | undefined {
  const eligible = keys.filter(key => key.is_active).map(credentialVerificationMeta).filter(meta => meta !== undefined);
  return eligible.find(meta => meta.variant === "destructive")?.label
    ?? eligible.find(meta => meta.variant !== "success")?.label ?? eligible[0]?.label;
}
