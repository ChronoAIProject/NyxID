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
  async preview(userCode: string) {
    return agentKeyPreviewSchema.parse(
      await requestJson("/auth/agent-key/preview", {
        method: "POST",
        body: { user_code: userCode },
        requiresAuth: false,
        retryOnAuthFailure: false,
      }),
    );
  },
  async options(userCode: string) {
    return agentKeyOptionsSchema.parse(
      await requestJson("/auth/agent-key/options", {
        method: "POST",
        body: { user_code: userCode },
      }),
    );
  },
  async approve(input: AgentKeyApprove) {
    return decisionSchema.parse(
      await requestJson("/auth/agent-key/approve", {
        method: "POST",
        body: agentKeyApproveSchema.parse(input),
      }),
    );
  },
  async deny(userCode: string) {
    return decisionSchema.parse(
      await requestJson("/auth/agent-key/deny", {
        method: "POST",
        body: { user_code: userCode },
      }),
    );
  },
};
