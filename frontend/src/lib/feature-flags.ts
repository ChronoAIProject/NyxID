/**
 * Feature flag catalog — the single client-side source of truth for flag keys.
 *
 * Mirrors the backend registry
 * (`backend/src/services/feature_flag_service.rs::FEATURE_FLAGS`); keys MUST
 * match the backend exactly. The backend owns authoritative metadata and
 * resolution — the client only needs the typed keys to gate UI.
 *
 * Reference flags by their catalog constant:
 *
 *   import { FEATURE_FLAG } from "@/lib/feature-flags";
 *   const on = useFeature(FEATURE_FLAG.AI_ASSISTANT, orgId);
 *
 * Adding a flag: add the key to the backend registry (deploy), then add one
 * entry here. `FeatureFlag` then narrows from `string` to the exact union, so
 * every call site becomes compile-time checked.
 */
export const FEATURE_FLAG = {
  INVITATION_CODE: "auth:invitation-code",
  AI_ASSISTANT: "experimental:ai-assistant",
  BILLING: "experimental:billing",
  AEVATAR_CHAT_WIRE_LOG: "experimental:aevatar-chat-wire-log",
  DIRECT_CHAT_ENGINE: "experimental:direct-chat-engine",
  ASSISTANT_VOICE: "assistant:voice",
  VOICE_GROK: "assistant:voice-grok",
  VOICE_GROK_PLATFORM: "assistant:voice-grok-platform",
  VOICE_OPENAI_PLATFORM: "assistant:voice-openai-platform",
  NYXAGENT_ENGINE: "assistant:nyxagent-engine",
  ORG_AGENTS: "assistant:org-agents",
  MACHINE_CAPABILITIES: "assistant:machine-capabilities",
  MACHINE_CONTEXTS: "assistant:machine-contexts",
  AGENT_OPERATION_SCOPES: "assistant:operation-scopes",
  AGENT_LEARNING: "assistant:agent-learning",
  NYXBOT_THREAD_FOLLOW: "nyxbot:thread-follow",
  NYXBOT_GATEWAY_LARK: "nyxbot:gateway-lark",
  NYXBOT_GATEWAY_FEISHU: "nyxbot:gateway-feishu",
  NYXBOT_GATEWAY_DISCORD: "nyxbot:gateway-discord",
  NYXBOT_GATEWAY_SLACK: "nyxbot:gateway-slack",
  NYXBOT_GATEWAY_WHATSAPP: "nyxbot:gateway-whatsapp",
  NYXBOT_GATEWAY_X: "nyxbot:gateway-x",
  NYXBOT_GATEWAY_AURINKO: "nyxbot:gateway-aurinko",
} as const;

type FeatureFlagKey = (typeof FEATURE_FLAG)[keyof typeof FEATURE_FLAG];

/**
 * Union of every known flag key. While the catalog is empty this is `string`
 * (nothing to constrain); it narrows to the exact union as flags are added.
 */
export type FeatureFlag = [FeatureFlagKey] extends [never]
  ? string
  : FeatureFlagKey;

/** All flag keys as an array (iteration, validation, tests). */
export const FEATURE_FLAG_KEYS = Object.values(FEATURE_FLAG) as FeatureFlag[];

/** Type guard: is this arbitrary string a known feature-flag key? */
export function isKnownFeatureFlag(key: string): key is FeatureFlag {
  return (FEATURE_FLAG_KEYS as readonly string[]).includes(key);
}
