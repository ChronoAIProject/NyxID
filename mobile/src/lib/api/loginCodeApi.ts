import { z } from "zod";
import { requestJson } from "./http";
import { agentKeyOptionsSchema, type AgentKeyApprove } from "./agentKeyLoginSchema";

const issuedSchema = z.object({ request_id: z.string(), code: z.string(), expires_at: z.string() });
const statusSchema = z.object({
  request_id: z.string(), status: z.enum(["pending", "redeemed", "expired", "cancelled", "revoked"]),
  expires_at: z.string(), redeemed_at: z.string().nullable(), client_label: z.string().nullable(),
  client_ip: z.string().nullable(), client_ip_attribution: z.string(), can_revoke: z.boolean(),
  auth_kind: z.enum(["agent_key", "account_session"]),
});
export type IssuedLoginCode = z.infer<typeof issuedSchema>;
export type LoginCodeStatus = z.infer<typeof statusSchema>;
export type LoginCodeGrant = { auth_kind: "account_session" } | {
  auth_kind: "agent_key"; selection: AgentKeyApprove["selection"]; credential_expires_at?: string;
};

export const loginCodeApi = {
  async options() { return agentKeyOptionsSchema.parse(await requestJson("/auth/login-code/options", { method: "POST", body: {} })); },
  async mint(grant: LoginCodeGrant) { return issuedSchema.parse(await requestJson("/auth/login-code", { method: "POST", body: grant })); },
  async status(id: string) { return statusSchema.parse(await requestJson(`/auth/login-code/${encodeURIComponent(id)}`)); },
  async cancel(id: string) { await requestJson(`/auth/login-code/${encodeURIComponent(id)}`, { method: "DELETE" }); },
  async revoke(id: string) { await requestJson(`/auth/login-code/${encodeURIComponent(id)}/revoke`, { method: "POST", body: {} }); },
};
