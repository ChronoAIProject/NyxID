import {
  act,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import {
  adapterBindingSchema,
  adapterDescriptorSchema,
  type AdapterBinding,
} from "@/schemas/channel-adapters";
import { ChannelAdapterBindings } from "./channel-adapter-bindings";

const mocks = vi.hoisted(() => ({
  bindings: [] as unknown[],
  error: null as Error | null,
  pending: false,
  create: vi.fn(),
  verify: vi.fn(),
  disable: vi.fn(),
  remove: vi.fn(),
  setup: vi.fn(),
  session: vi.fn(),
  endSession: vi.fn(),
  reset: vi.fn(),
  refetch: vi.fn(),
}));
const owner = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const telnyx = "11111111-1111-4111-8111-111111111111";
const twilio = "22222222-2222-4222-8222-222222222222";
const catalogId = "33333333-3333-4333-8333-333333333333";

function descriptor(id: string) {
  const unavailable =
    id === "telnyx.conversation_relay" || id.startsWith("elevenlabs.");
  return adapterDescriptorSchema.parse({
    adapter_id: id,
    contract_version: 1,
    operations: ["stream_reply"],
    messaging: {
      inbound_text: false,
      outbound_text: false,
      max_text_bytes: null,
      delivery: "unsupported",
    },
    conversation: {
      final_transcripts: true,
      streamed_output: true,
      interruption: true,
      explicit_cancel: false,
      playback_evidence: "interrupted_prefix",
    },
    instructions: {
      external_nyxbot: true,
      nyxbot_refresh: "between_turns",
      provider_prompt_replacement: "unsupported",
    },
    lifecycle: {
      max_session_seconds: 1800,
      reconnect: false,
      automatic_effect_replay: false,
    },
    setup: {
      agent: "nyxbot_specialist",
      carrier: "external",
      speech: "external",
      documentation: "https://example.com/setup",
      credential_modes: [
        { kind: "connection", implemented: true, prerequisite: null },
        {
          kind: "platform",
          implemented: !unavailable,
          prerequisite: unavailable ? "provider_limit_unavailable" : null,
        },
      ],
    },
  });
}
vi.mock("@/hooks/use-channel-adapters", () => ({
  useChannelAdapterCatalog: () => ({
    data: [
      descriptor("telnyx.conversation_relay"),
      descriptor("telnyx.messaging"),
      descriptor("twilio.conversation_relay"),
      descriptor("elevenlabs.custom_llm"),
    ],
    error: null,
    refetch: mocks.refetch,
  }),
  useChannelAdapterBindings: () => ({
    data: mocks.bindings,
    error: mocks.error,
    isPending: mocks.pending,
    isSuccess: !mocks.error && !mocks.pending,
    refetch: mocks.refetch,
  }),
  useChannelAdapterActions: () => ({
    create: { mutateAsync: mocks.create, isPending: false },
    verify: { mutateAsync: mocks.verify, isPending: false },
    disable: { mutateAsync: mocks.disable, isPending: false },
    remove: { mutateAsync: mocks.remove, isPending: false },
    customSetup: { mutateAsync: mocks.setup, reset: mocks.reset },
    customSession: { mutateAsync: mocks.session, reset: mocks.reset },
    endSession: { mutateAsync: mocks.endSession, isPending: false },
  }),
}));
vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    data: [
      {
        id: telnyx,
        label: "My Telnyx",
        catalog_service_id: catalogId,
        catalog_service_slug: "api-telnyx",
        is_active: true,
        status: "active",
        platform_key_available: true,
      },
      {
        id: "44444444-4444-4444-8444-444444444444",
        label: "Same platform twice",
        catalog_service_id: catalogId,
        catalog_service_slug: "api-telnyx",
        is_active: true,
        status: "active",
        platform_key_available: true,
      },
      {
        id: twilio,
        label: "My Twilio",
        catalog_service_id: "55555555-5555-4555-8555-555555555555",
        catalog_service_slug: "api-twilio",
        is_active: true,
        status: "active",
        platform_key_available: true,
      },
      {
        id: "blocked",
        label: "Inaccessible organization",
        catalog_service_id: catalogId,
        catalog_service_slug: "api-telnyx",
        is_active: true,
        status: "active",
        credential_source: { type: "org", allowed: false },
      },
    ],
  }),
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({
    data: {
      agents: [
        {
          id: "faq-specialist",
          name: "FAQ",
          kind: "specialist",
          status: "idle",
          owner_id: owner,
        },
        { id: "main", name: "Main NyxBot", kind: "nyxbot", status: "idle" },
        {
          id: "destroyed",
          name: "Destroyed",
          kind: "specialist",
          status: "destroyed",
        },
        {
          id: "org",
          name: "Organization",
          kind: "specialist",
          status: "idle",
          owner_kind: "org",
        },
        {
          id: "other",
          name: "Another person's specialist",
          kind: "specialist",
          status: "idle",
          owner_id: "other",
        },
      ],
    },
  }),
}));

function setOwner(id = owner) {
  useAuthStore.getState().setUser({
    id,
    email: "owner@example.com",
    display_name: "Owner",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: "2026-10-17T00:00:00Z",
  });
}
function binding(changes: Partial<AdapterBinding> = {}) {
  return adapterBindingSchema.parse({
    id: "66666666-6666-4666-8666-666666666666",
    label: "FAQ phone",
    adapter_id: "telnyx.conversation_relay",
    contract_version: 1,
    revision: 1,
    agent_id: "faq-specialist",
    credential: { kind: "connection", user_service_id: telnyx },
    account_id: "account",
    resource_id: "number",
    application_id: "app",
    phone_number: "+14155550123",
    voice: "Telnyx.Ultra.Callie",
    language: "en-US",
    enabled: true,
    verified_at: "2026-10-17T00:00:00Z",
    webhook_url: "https://example.com/webhook",
    availability: {
      available: true,
      reason: null,
      operations: [
        {
          operation: "stream_reply",
          implemented: true,
          available: true,
          reason: null,
        },
      ],
    },
    ...changes,
  });
}
beforeEach(() => {
  vi.clearAllMocks();
  setOwner();
  mocks.bindings = [];
  mocks.error = null;
  mocks.pending = false;
  mocks.create.mockResolvedValue({});
  mocks.verify.mockResolvedValue({});
  mocks.disable.mockResolvedValue(undefined);
  mocks.remove.mockResolvedValue(undefined);
  mocks.endSession.mockResolvedValue(undefined);
  mocks.session.mockResolvedValue({
    session_id: "77777777-7777-4777-8777-777777777777",
    extra_body: { nyxid_session_token: "fixture-session-token-only" },
  });
  mocks.setup.mockResolvedValue({
    api_key: "fixture-secret-only",
    url: "https://example.com/custom",
  });
});
afterEach(cleanup);
async function select(
  user: ReturnType<typeof userEvent.setup>,
  name: string,
  option: string,
) {
  await user.click(screen.getByRole("combobox", { name }));
  await user.click(screen.getByRole("option", { name: option }));
}
async function openForm() {
  const user = userEvent.setup();
  render(<ChannelAdapterBindings />);
  await user.click(
    screen.getByRole("button", { name: "Add phone or speech connection" }),
  );
  return user;
}
async function fillTelnyx(user: ReturnType<typeof userEvent.setup>) {
  await user.type(screen.getByLabelText("Connection name"), "FAQ line");
  await select(user, "Specialist", "FAQ");
  await select(user, "Service connection", "My Telnyx");
  await user.type(screen.getByLabelText("Provider account ID"), "account");
  await user.type(screen.getByLabelText("Phone number"), "+14155550123");
  await user.type(
    screen.getByLabelText("Provider application / messaging profile ID"),
    "application",
  );
  await user.type(
    screen.getByLabelText("Provider number resource ID"),
    "number",
  );
  await user.type(
    screen.getByLabelText("Telnyx Ed25519 public key"),
    "public-key",
  );
}

it("only offers the owner's active specialists and blocks unsupported platform voice", async () => {
  const user = await openForm();
  await user.click(screen.getByRole("combobox", { name: "Specialist" }));
  expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
    "FAQ",
  ]);
  await user.click(screen.getByRole("option", { name: "FAQ" }));
  await user.click(screen.getByRole("combobox", { name: "Credentials" }));
  expect(
    screen.getByRole("option", { name: "Explicit platform account" }),
  ).toHaveAttribute("data-disabled");
  await user.keyboard("{Escape}");
  await user.click(
    screen.getByRole("combobox", { name: "Service connection" }),
  );
  expect(
    screen.queryByRole("option", { name: "Inaccessible organization" }),
  ).not.toBeInTheDocument();
});

it("submits the selected specialist and connection only after valid setup", async () => {
  const user = await openForm();
  expect(
    screen.getByRole("button", { name: "Verify and connect" }),
  ).toBeDisabled();
  await fillTelnyx(user);
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Verify and connect" }),
    ).toBeEnabled(),
  );
  await user.click(screen.getByRole("button", { name: "Verify and connect" }));
  expect(mocks.create).toHaveBeenCalledExactlyOnceWith(
    expect.objectContaining({
      agent_id: "faq-specialist",
      credential: { kind: "connection", user_service_id: telnyx },
      phone_number: "+14155550123",
    }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
});

it("clears a stale credential when switching providers", async () => {
  const user = await openForm();
  await fillTelnyx(user);
  await select(user, "Profile", "Twilio ConversationRelay");
  expect(
    screen.getByRole("combobox", { name: "Service connection" }),
  ).toHaveTextContent("Choose an available service");
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Verify and connect" }),
    ).toBeDisabled(),
  );
  await select(user, "Service connection", "My Twilio");
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Verify and connect" }),
    ).toBeEnabled(),
  );
  await user.click(screen.getByRole("button", { name: "Verify and connect" }));
  expect(mocks.create).toHaveBeenCalledWith(
    expect.objectContaining({
      adapter_id: "twilio.conversation_relay",
      credential: { kind: "connection", user_service_id: twilio },
      voice: "en-US-Standard-A",
    }),
  );
});

it("deduplicates platform services and clears the credential on mode changes", async () => {
  const user = await openForm();
  await select(user, "Profile", "Telnyx SMS");
  await select(user, "Service connection", "My Telnyx");
  await select(user, "Credentials", "Explicit platform account");
  expect(
    screen.getByRole("combobox", { name: "Platform service" }),
  ).toHaveTextContent("Choose an available service");
  await user.click(screen.getByRole("combobox", { name: "Platform service" }));
  expect(screen.getAllByRole("option")).toHaveLength(1);
  await user.click(screen.getAllByRole("option")[0]!);
  await select(user, "Credentials", "Your service connection");
  expect(
    screen.getByRole("combobox", { name: "Service connection" }),
  ).toHaveTextContent("Choose an available service");
});

it("shows an actionable unavailable reason and prevents removal while enabled", () => {
  mocks.bindings = [
    binding({
      credential: { kind: "platform", catalog_service_id: catalogId },
      availability: {
        available: false,
        reason: "provider_limit_unavailable",
        operations: [],
      },
    }),
  ];
  render(<ChannelAdapterBindings />);
  expect(screen.getByText("Setup needed")).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent(
    "Platform calling is unavailable",
  );
  expect(
    screen.queryByRole("button", { name: "Remove connection" }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Disable" })).toBeEnabled();
});

it("retains cleanup conflicts so the owner can retry removal", async () => {
  mocks.bindings = [binding({ enabled: false })];
  mocks.remove.mockRejectedValueOnce(new Error("Session cleanup is pending."));
  const user = userEvent.setup();
  render(<ChannelAdapterBindings />);
  await user.click(screen.getByRole("button", { name: "Remove connection" }));
  expect(
    await screen.findByText("Session cleanup is pending."),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Remove connection" }),
  ).toBeEnabled();
});

it("clears delivered Custom LLM secrets on close and account change", async () => {
  mocks.bindings = [
    binding({ adapter_id: "elevenlabs.custom_llm", phone_number: "" }),
  ];
  const user = userEvent.setup();
  render(<ChannelAdapterBindings />);
  await user.click(
    screen.getByRole("button", { name: "Show Custom LLM setup" }),
  );
  expect(await screen.findByText(/fixture-secret-only/)).toBeInTheDocument();
  expect(mocks.reset).toHaveBeenCalledOnce();
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }),
  );
  expect(screen.queryByText(/fixture-secret-only/)).not.toBeInTheDocument();
  await user.click(
    screen.getByRole("button", { name: "Show Custom LLM setup" }),
  );
  expect(await screen.findByText(/fixture-secret-only/)).toBeInTheDocument();
  act(() => setOwner("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"));
  expect(screen.queryByText(/fixture-secret-only/)).not.toBeInTheDocument();
});

it("ends the issued guest session after its secret dialog closes", async () => {
  const row = binding({
    adapter_id: "elevenlabs.custom_llm",
    phone_number: "",
  });
  mocks.bindings = [row];
  const user = userEvent.setup();
  render(<ChannelAdapterBindings />);
  await user.click(
    screen.getByRole("button", { name: "Create guest session" }),
  );
  expect(await screen.findByText(/fixture-session-token-only/)).toBeVisible();
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }),
  );
  expect(
    screen.queryByText(/fixture-session-token-only/),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "End guest session" }));
  expect(mocks.endSession).toHaveBeenCalledExactlyOnceWith({
    id: row.id,
    sessionId: "77777777-7777-4777-8777-777777777777",
  });
  expect(
    screen.queryByRole("button", { name: "End guest session" }),
  ).not.toBeInTheDocument();
});
