import { z } from "zod";

import { requestJson } from "./http";

const authDevicePreviewSchema = z.object({
  client_label: z.string().nullable(),
  client_user_agent: z.string().nullable(),
  client_ip: z.string().nullable(),
  initiated_at: z.string().datetime(),
  expires_at: z.string().datetime(),
  status: z.enum(["pending", "approved", "denied", "expired", "delivered"]),
});

const authDeviceDecisionSchema = z.object({
  ok: z.literal(true),
});

export type AuthDevicePreview = z.infer<typeof authDevicePreviewSchema>;

export async function previewAuthDeviceRequest(userCode: string): Promise<AuthDevicePreview> {
  const response = await requestJson<unknown>("/auth/device/preview", {
    method: "POST",
    body: { user_code: userCode },
    requiresAuth: false,
    retryOnAuthFailure: false,
  });

  return authDevicePreviewSchema.parse(response);
}

export async function approveAuthDeviceRequest(userCode: string): Promise<void> {
  const response = await requestJson<unknown>("/auth/device/approve", {
    method: "POST",
    body: { user_code: userCode },
  });

  authDeviceDecisionSchema.parse(response);
}

export async function denyAuthDeviceRequest(userCode: string): Promise<void> {
  const response = await requestJson<unknown>("/auth/device/deny", {
    method: "POST",
    body: { user_code: userCode },
  });

  authDeviceDecisionSchema.parse(response);
}
