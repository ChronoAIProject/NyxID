import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { ManagedEligibility, ManagedLine, ManagedQuote } from "@/schemas/managed-lines";
import { ManagedPhoneLines } from "./managed-phone-lines";

const ids = {
  owner: "3f0b8a3e-6c55-4b1e-9a61-1c2f0e9a7d10",
  line: "0b8f4b44-3d73-4d0c-8f59-5d1c4f7b2a61",
  free: "1c9f5c55-4e84-4e1d-9f6a-6e2d5f8c3b72",
  claim: "8f8a6e93-1f0c-4f3e-9d77-2a8b9b0b6c3a",
  quote: "a7c0f5d2-5b8e-4d3a-8c5f-0e1d2c3b4a59",
  agent: "d1e2f3a4-b5c6-4d7e-8f90-a1b2c3d4e5f6",
};
const state = vi.hoisted(() => ({
  eligibility: null as ManagedEligibility | null,
  lines: [] as ManagedLine[],
  release: vi.fn(),
  claim: vi.fn(),
  quote: vi.fn(),
}));
const mutation = (fn: (input: never) => unknown) => ({
  mutateAsync: fn,
  isPending: false,
});
vi.mock("@/hooks/use-managed-lines", () => ({
  useManagedEligibility: () => ({
    data: state.eligibility,
    isPending: false,
    isSuccess: true,
    error: null,
    refetch: vi.fn(),
  }),
  useManagedLines: () => ({
    data: state.lines,
    isPending: false,
    error: null,
    hasNextPage: false,
    refetch: vi.fn(),
  }),
  useManagedLineActions: () => ({
    quote: mutation(state.quote),
    claim: mutation(state.claim),
    coverage: mutation(vi.fn()),
    release: mutation(state.release),
    acceptTerms: mutation(vi.fn()),
    setVoice: mutation(vi.fn()),
    confirmations: mutation(vi.fn()),
  }),
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({
    data: {
      agents: [
        { id: ids.agent, kind: "specialist", name: "Bookings", status: "idle", owner_kind: "person", owner_id: "owner" },
      ],
    },
  }),
}));
vi.mock("@/hooks/use-account-panel", () => ({ useAccountPanel: () => ({ open: vi.fn() }) }));

const eligibility: ManagedEligibility = {
  id: ids.claim,
  owner_id: ids.owner,
  business_legal_name: "Jolly Chili Pte Ltd",
  registration_id: "201912345K",
  jurisdiction: "SG",
  allowed_use_case: "restaurant bookings",
  sms_sender_path: "long_code",
  sms_registration_status: "approved",
  required_terms: { managed_line: "2026-11", elevenlabs_eua: null },
  accepted_terms: { managed_line: "2026-11", elevenlabs_eua: null },
  terms_accepted_at: "2026-11-02T00:00:00Z",
  premium_allowed: false,
  max_lines: 2,
  monthly_spend_limit: null,
  revision: 3,
  approved_at: "2026-11-01T00:00:00Z",
  revoked_at: null,
};
const claimed: ManagedLine = {
  id: ids.line,
  phone_number: "+6560000001",
  country: "SG",
  state: "claimed",
  voice: true,
  sms: true,
  standard_voice_options: [{ voice: "Telnyx.Ultra.Callie", language: "en-US" }],
  claim: {
    id: ids.claim,
    generation: 1,
    state: "claimed",
    suspension: null,
    auto_renew: true,
    standard_voice: { voice: "Telnyx.Ultra.Callie", language: "en-US" },
    covered_from: "2026-11-01T00:00:00Z",
    covered_until: "2026-12-01T00:00:00Z",
    period_index: 0,
    renewal_quote: null,
    agent_id: ids.agent,
    voice_tier: "standard",
    sms_enabled: true,
    caller_confirmations: false,
    voice_binding_id: ids.quote,
    voice_binding_revision: 2,
  },
};
const available: ManagedLine = {
  ...claimed,
  id: ids.free,
  phone_number: "+6560000002",
  state: "available",
  sms: false,
  claim: null,
};
const quote: ManagedQuote = {
  id: ids.quote,
  line_id: ids.free,
  kind: { kind: "claim" },
  rental: { metric: "phone_number_days", credits_per_unit: "0.5", revision: "r", purpose: "rental" },
  usage: [{ metric: "voice_seconds", credits_per_unit: "0.001", revision: "r", purpose: "calls" }],
  standard_voice_options: [{ voice: "Telnyx.Ultra.Callie", language: "en-US" }],
  terms: { managed_line: "2026-11", elevenlabs_eua: null },
  accept_before: "2026-11-20T00:00:00Z",
  period_days: 30,
  paid_period_refundable: false,
};

beforeEach(() => {
  vi.clearAllMocks();
  state.eligibility = eligibility;
  state.lines = [claimed, available];
  useAuthStore.getState().setUser({
    id: "owner",
    email: "owner@example.com",
    display_name: "Owner",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: "2026-11-01T00:00:00Z",
  });
});
afterEach(cleanup);

it("explains approval instead of offering numbers to unapproved merchants", () => {
  state.eligibility = null;
  state.lines = [];
  render(<ManagedPhoneLines />);
  expect(screen.getByText("Merchant approval required")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Get a NyxID number" })).not.toBeInTheDocument();
});

it("requires current terms before a number can be claimed", () => {
  state.eligibility = { ...eligibility, accepted_terms: null };
  render(<ManagedPhoneLines />);
  expect(screen.getByRole("button", { name: "Get a NyxID number" })).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Review and accept the current terms" }),
  ).toBeInTheDocument();
});

it("shows coverage and retries a release with the same request ID", async () => {
  const user = userEvent.setup({ delay: null });
  state.release.mockRejectedValueOnce(new Error("network")).mockResolvedValueOnce({});
  render(<ManagedPhoneLines />);
  const line = screen.getByRole("region", { name: "+6560000001" });
  expect(within(line).getByText(/the end time is not included/)).toBeInTheDocument();
  await user.click(within(line).getByRole("button", { name: "Release this number" }));
  const dialog = await screen.findByRole("dialog");
  await user.click(within(dialog).getByRole("button", { name: "Release number" }));
  await within(dialog).findByText("network");
  await user.click(within(dialog).getByRole("button", { name: "Release number" }));
  await waitFor(() => expect(state.release).toHaveBeenCalledTimes(2));
  const [first, second] = state.release.mock.calls.map((call) => call[0] as { request_id: string });
  expect(first!.request_id).toMatch(/^[0-9a-f-]{36}$/);
  expect(second!.request_id).toBe(first!.request_id);
  expect(state.release.mock.calls[0]?.[0]).toMatchObject({ line_id: ids.line, claim_id: ids.claim });
});

it("claims only after explicit price and terms consent, with the quoted terms", async () => {
  const user = userEvent.setup({ delay: null });
  state.quote.mockResolvedValue(quote);
  state.claim.mockResolvedValue({});
  render(<ManagedPhoneLines />);
  await user.click(screen.getByRole("button", { name: "Get a NyxID number" }));
  await user.click(await screen.findByRole("button", { name: "See prices" }));
  expect(state.quote).toHaveBeenCalledWith({ lineId: ids.free, kind: { kind: "claim" } });
  const dialog = await screen.findByRole("dialog", { name: "+6560000002" });
  expect(within(dialog).getByText(/15 credits per 30 days/)).toBeInTheDocument();
  await user.click(within(dialog).getByRole("combobox", { name: "Specialist" }));
  await user.click(await screen.findByRole("option", { name: "Bookings" }));
  const submit = within(dialog).getByRole("button", { name: "Pay and get this number" });
  expect(submit).toBeDisabled();
  for (const box of within(dialog).getAllByRole("checkbox")) await user.click(box);
  await waitFor(() => expect(submit).toBeEnabled());
  await user.click(submit);
  await waitFor(() => expect(state.claim).toHaveBeenCalledTimes(1));
  expect(state.claim.mock.calls[0]?.[0]).toMatchObject({
    quote_id: ids.quote,
    line_id: ids.free,
    agent_id: ids.agent,
    voice_tier: "standard",
    standard_voice: { voice: "Telnyx.Ultra.Callie", language: "en-US" },
    enable_sms: false,
    // Untouched, the recurring-charge consent stays off.
    auto_renew: false,
    accepted_terms_versions: quote.terms,
  });
});

it("submits automatic renewal only after the merchant turns it on", async () => {
  const user = userEvent.setup({ delay: null });
  state.quote.mockResolvedValue(quote);
  state.claim.mockResolvedValue({});
  render(<ManagedPhoneLines />);
  await user.click(screen.getByRole("button", { name: "Get a NyxID number" }));
  await user.click(await screen.findByRole("button", { name: "See prices" }));
  const dialog = await screen.findByRole("dialog", { name: "+6560000002" });
  const renew = within(dialog).getByRole("switch", { name: "Renew automatically at this price" });
  expect(renew).not.toBeChecked();
  await user.click(within(dialog).getByRole("combobox", { name: "Specialist" }));
  await user.click(await screen.findByRole("option", { name: "Bookings" }));
  await user.click(renew);
  for (const box of within(dialog).getAllByRole("checkbox")) await user.click(box);
  const submit = within(dialog).getByRole("button", { name: "Pay and get this number" });
  await waitFor(() => expect(submit).toBeEnabled());
  await user.click(submit);
  await waitFor(() => expect(state.claim).toHaveBeenCalledTimes(1));
  expect(state.claim.mock.calls[0]?.[0]).toMatchObject({ auto_renew: true });
});
