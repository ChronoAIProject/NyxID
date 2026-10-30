import { z } from "zod";
import { requestJson } from "./http";
import type { AccountProfile } from "./types";
import {
  mfaConfirmSchema,
  mfaSetupSchema,
  sessionSchema,
  totpCodeSchema,
  updateProfileSchema,
  type UpdateProfileInput,
} from "./accountSchema";

function parseResponse<T>(schema: z.ZodType<T>, payload: unknown): T {
  const result = schema.safeParse(payload);
  // Keep secret-bearing server payloads out of error messages and telemetry.
  if (!result.success)
    throw new Error("NyxID returned an unexpected response. Please try again.");
  return result.data;
}

export const accountApi = {
  getProfile: (signal?: AbortSignal) =>
    requestJson<AccountProfile>("/users/me", { signal }),
  updateProfile: (input: UpdateProfileInput) =>
    requestJson<{
      id: string;
      email: string;
      display_name: string | null;
      avatar_url: string | null;
      message: string;
    }>("/users/me", {
      method: "PUT",
      body: updateProfileSchema.parse(input),
    }),
  async listSessions(signal?: AbortSignal) {
    return parseResponse(
      z.array(sessionSchema),
      await requestJson<unknown>("/sessions", { signal }),
    );
  },
  revokeSession: (id: string) =>
    requestJson<void>(`/sessions/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  async setupMfa() {
    return parseResponse(
      mfaSetupSchema,
      await requestJson<unknown>("/auth/mfa/setup", { method: "POST" }),
    );
  },
  async confirmMfa(code: string) {
    return parseResponse(
      mfaConfirmSchema,
      await requestJson<unknown>("/auth/mfa/confirm", {
        method: "POST",
        body: { code: totpCodeSchema.parse(code) },
        retryOnAuthFailure: false,
        invalidateSessionOnAuthFailure: false,
      }),
    );
  },
  disableMfa: (password: string) =>
    requestJson<{ message: string }>("/auth/mfa/disable", {
      method: "POST",
      body: { password },
      retryOnAuthFailure: false,
      invalidateSessionOnAuthFailure: false,
    }),
};
