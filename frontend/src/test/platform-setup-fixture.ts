import {
  blankFacts,
  type OwnedBinding,
  type PlatformSetup,
} from "../schemas/platform-onboarding";
import type { User } from "../types/api";

export const setupActor: User = {
  id: "11111111-1111-4111-8111-111111111111",
  email: "draft-owner@example.test",
  display_name: "Draft owner",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: false,
  is_active: true,
  created_at: "2026-11-01T00:00:00Z",
};

export const setupBinding: OwnedBinding = {
  id: "22222222-2222-4222-8222-222222222222",
  generation: 7,
  platform_config_id: "33333333-3333-4333-8333-333333333333",
  manager_bot_id: "100",
  telegram_sender_id: "200",
  expires_at: "2099-01-01T00:00:00Z",
};

export function platformSetup(
  overrides: Partial<PlatformSetup> = {},
): PlatformSetup {
  return {
    id: "44444444-4444-4444-8444-444444444444",
    template: "telegram-assistant",
    template_version: 1,
    revision: 3,
    state: "draft",
    facts: { ...blankFacts(), name: "Telegram cafe", timezone: "UTC" },
    limits: {
      daily_credits: "1",
      turns_per_hour: 10,
      concurrent_turns: 1,
      max_input_chars: 1024,
      max_output_tokens: 256,
    },
    agent_id: null,
    channel_bot_id: null,
    creation_request_id: null,
    observed_telegram_bot_id: null,
    publication_revision: null,
    approved: false,
    delivery_tested: false,
    expires_at: "2099-01-01T00:00:00Z",
    updated_at: "2026-11-01T00:00:00Z",
    ...overrides,
  };
}

export const setupTemplates = [
  {
    id: "telegram-assistant",
    version: 1,
    name: "Telegram assistant",
    description: "A fixture starter for a private assistant",
    fields: ["name", "timezone"],
    runtime_connector_grants: [],
  },
];
