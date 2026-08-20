import { z } from "zod";

import { requestJson } from "./http";
import {
  parseAuthDevicePreview,
  type AuthDevicePreview,
} from "./authDeviceSchema";

const authDeviceDecisionSchema = z.object({
  ok: z.literal(true),
});

export type { AuthDevicePreview } from "./authDeviceSchema";

export async function previewAuthDeviceRequest(userCode: string): Promise<AuthDevicePreview> {
  const response = await requestJson<unknown>("/auth/device/preview", {
    method: "POST",
    body: { user_code: userCode },
    requiresAuth: false,
    retryOnAuthFailure: false,
  });

  return parseAuthDevicePreview(response);
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
