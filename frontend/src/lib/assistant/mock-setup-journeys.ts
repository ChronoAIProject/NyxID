import type {
  ConnectLinkPreview,
  ConnectLinkStatusResponse,
} from "@/schemas/connect-links";
import type {
  ChannelPlatformDescriptor,
  CreateChannelBotResponse,
} from "@/types/channels";
import { useAssistantMockScenariosStore } from "@/stores/assistant-mock-scenarios-store";

const STORAGE = "nyxid.mock-setup-journeys.v1";
const LEGACY_TOKEN = "nyx_clk_fixture_github";
interface MockLink {
  id: string;
  token: string;
  status: "pending" | "completed" | "cancelled";
  created_at: string;
  expires_at: string;
  completed_at: string | null;
}
interface MockSetupState {
  links: MockLink[];
  bots: CreateChannelBotResponse[];
}

function state(): MockSetupState {
  return JSON.parse(
    sessionStorage.getItem(STORAGE) ?? '{"links":[],"bots":[]}',
  ) as MockSetupState;
}

function save(value: MockSetupState) {
  sessionStorage.setItem(STORAGE, JSON.stringify(value));
}

function newLink(token?: string): MockLink {
  const id = crypto.randomUUID();
  return {
    id,
    token: token ?? `nyx_clk_fixture_${id}`,
    status: "pending",
    created_at: new Date().toISOString(),
    expires_at: new Date(Date.now() + 3_600_000).toISOString(),
    completed_at: null,
  };
}

export function createMockConnectReply(): string {
  const current = state();
  const link = newLink();
  current.links.push(link);
  save(current);
  return `Connect GitHub to continue. [Connect GitHub](/connect/${link.token})`;
}

export function mockCreatedBotCount(): number {
  return state().bots.length;
}

const TELEGRAM: ChannelPlatformDescriptor = {
  platform: "telegram",
  display_name: "Telegram bot token",
  enabled: true,
  managed_only: false,
  managed_only_message: "",
  ingestion: { mode: "webhook" },
  registration: {
    fields: [
      {
        name: "bot_token",
        label: "Bot token",
        secret: true,
        required: true,
        patchable: true,
        clearable: false,
        storage: "bot_token",
        webhook_secret: false,
        platform_fallback: null,
        hint: "Local demo: enter demo-bot-token. No Telegram request is sent.",
      },
    ],
    extra_fields: [],
    token_fields: ["bot_token"],
    required_suffix: "",
    automatic_webhook: true,
    webhook_ingestion: true,
    webhook_secret_label: null,
    create_response_status: "active",
    setup_instructions: [
      "This local demo waits for you to enter a dummy token and click Add Bot.",
    ],
  },
  managed_onboarding: null,
  platform_credentials: null,
  capabilities: {
    media: { inbound: [], outbound: [] },
    initiated_send: true,
    reply_to: true,
    thread: true,
    edit: true,
  },
  webhook_path: "/api/v1/webhooks/channel/telegram/{bot_id}",
};

function preview(link: MockLink): ConnectLinkPreview {
  return {
    service_name: "GitHub",
    service_slug: "github",
    label: "NyxBot GitHub connection",
    requested_by: "NyxBot",
    scopes: ["repo"],
    status: link.status,
    created_at: link.created_at,
    expires_at: link.expires_at,
    connect_method: "api_key",
    auth_key_name: "Personal access token",
    credential_mode: "api_key",
    has_platform_oauth_credentials: false,
    requires_gateway_url: false,
    api_key_url: null,
    callback_url: null,
    use_platform_key: false,
    api_key_instructions:
      "Local demo: enter demo-github-token, then click Approve & connect. No GitHub request is sent.",
  };
}

function status(link: MockLink): ConnectLinkStatusResponse {
  return {
    id: link.id,
    status: link.status,
    service_name: "GitHub",
    service_slug: "github",
    scopes: ["repo"],
    expires_at: link.expires_at,
    completed_at: link.completed_at,
    connected_service:
      link.status === "completed" ? { id: link.id, slug: "github" } : null,
    callback_url: null,
  };
}

/** Development API fixtures; only explicit form mutations finish a setup. */
export function mockSetupResponse(
  path: string,
  method: string,
  body?: unknown,
): unknown {
  const request = (body && typeof body === "object" ? body : {}) as Record<
    string,
    unknown
  >;
  if (path.startsWith("/connect-links/")) {
    const current = state();
    let link = current.links.find(
      (row) =>
        row.token === request.token || path === `/connect-links/${row.id}`,
    );
    if (!link && request.token === LEGACY_TOKEN) {
      link = newLink(LEGACY_TOKEN);
      current.links.push(link);
      save(current);
    }
    if (!link)
      throw new Error(
        "Unknown demo link. Ask the mock chat for a new GitHub connection.",
      );
    if (method === "GET") return status(link);
    if (method === "POST" && path === "/connect-links/preview")
      return preview(link);
    if (method === "POST" && path === "/connect-links/cancel") {
      if (link.status === "pending") link.status = "cancelled";
      save(current);
      return status(link);
    }
    if (method === "POST" && path === "/connect-links/complete") {
      if (link.status === "cancelled")
        throw new Error(
          "This demo connection was cancelled. Request a new link.",
        );
      if (typeof request.credential !== "string" || !request.credential.trim())
        throw new Error("Enter a dummy token to connect.");
      link.status = "completed";
      link.completed_at = new Date().toISOString();
      save(current);
      useAssistantMockScenariosStore.getState().connectService("api-github");
      return {
        id: link.id,
        status: "completed",
        service_slug: "github",
        user_service_id: link.id,
        callback_url: null,
      };
    }
    throw new Error("This operation is not part of the local connection demo.");
  }
  if (method === "GET" && path === "/channel-platforms")
    return { platforms: [TELEGRAM] };
  if (method === "POST" && path === "/channel-bots/telegram/profile") {
    return {
      username: "helper_bot",
      display_name: "Helper Bot",
      label: "Helper Bot",
    };
  }
  if (method === "POST" && path === "/channel-bots") {
    if (
      request.platform !== "telegram" ||
      typeof request.bot_token !== "string" ||
      !request.bot_token.trim()
    ) {
      throw new Error("Enter a dummy Telegram bot token to continue.");
    }
    const current = state();
    const bot: CreateChannelBotResponse = {
      id: crypto.randomUUID(),
      platform: "telegram",
      label: String(request.label || "Helper"),
      platform_bot_username: "helper_bot",
      status: "active",
      credential_source: "user",
      setup_instructions: [
        "Demo bot created. No external platform call was made.",
      ],
    };
    current.bots.push(bot);
    save(current);
    return bot;
  }
  return undefined;
}
