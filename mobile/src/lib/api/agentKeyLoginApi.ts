import { z } from "zod";
import { requestJson } from "./http";
import {
  agentKeyApproveSchema,
  agentKeyOptionsSchema,
  agentKeyPreviewSchema,
  type AgentKeyApprove,
} from "./agentKeyLoginSchema";

const decisionSchema = z.object({ ok: z.literal(true) });
export const agentKeyLoginApi = {
  async preview(userCode: string, flow: "agent-key" | "device" = "agent-key") {
    return agentKeyPreviewSchema.parse(
      { requested_profile: null, interval: 5, ...await requestJson<Record<string, unknown>>(`/auth/${flow}/preview`, {
        method: "POST",
        body: { user_code: userCode },
        requiresAuth: false,
        retryOnAuthFailure: false,
      }) },
    );
  },
  async options(userCode: string, flow: "agent-key" | "device" = "agent-key") {
    return agentKeyOptionsSchema.parse(
      await requestJson(`/auth/${flow}/options`, {
        method: "POST",
        body: { user_code: userCode },
      }),
    );
  },
  async approve(input: AgentKeyApprove, flow: "agent-key" | "device" = "agent-key") {
    return decisionSchema.parse(
      await requestJson(`/auth/${flow}/${flow === "device" ? "approve-agent-key" : "approve"}`, {
        method: "POST",
        body: agentKeyApproveSchema.parse(input),
      }),
    );
  },
  async deny(userCode: string, flow: "agent-key" | "device" = "agent-key") {
    return decisionSchema.parse(
      await requestJson(`/auth/${flow}/deny`, {
        method: "POST",
        body: { user_code: userCode },
      }),
    );
  },
};
