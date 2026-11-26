import { z } from "zod";

import { requestJson } from "./http";
import {
  parseAuthDevicePreview,
  stagingQrBindingSchema,
  type StagingQrBinding,
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


export async function previewStagingDevice(userCode: string, binding?: StagingQrBinding): Promise<AuthDevicePreview> {
  const result = await requestJson<unknown>("/auth/device/staging", {
    method: "POST",
    body: { action: "preview", user_code: userCode, binding },
    expectedAccountId: binding?.production_user,
  });
  const response = z.object({ preview: z.unknown(), binding: stagingQrBindingSchema }).parse(result);
  return { ...parseAuthDevicePreview(response.preview), supports_grant_choice: false, stagingBinding: response.binding };
}

export async function decideStagingDevice(action: "approve" | "deny", userCode: string, binding: StagingQrBinding) {
  const response = await requestJson<unknown>("/auth/device/staging", {
    method: "POST",
    body: { action, user_code: userCode, binding },
    expectedAccountId: binding.production_user,
  });
  authDeviceDecisionSchema.parse(response);
}
