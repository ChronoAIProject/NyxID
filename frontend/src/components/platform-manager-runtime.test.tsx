import { beforeEach, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import {
  AiRuntimeForm,
  ManagerRuntimeSettings,
} from "./platform-manager-runtime";
import { ApiError } from "@/lib/api-client";
import type { ManagerRuntime } from "@/schemas/platform-runtime";
import type { PlatformProfile } from "@/schemas/platform-onboarding";
const sponsor = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const { query, mutateAsync } = vi.hoisted(() => ({
  query: {
    data: {} as ManagerRuntime,
    isPending: false,
    error: null,
    refetch: vi.fn(),
  },
  mutateAsync: vi.fn(),
}));
vi.mock("@/hooks/use-platform-runtime", () => ({
  useManagerRuntime: () => query,
  useManagerRuntimeAction: () => ({ mutateAsync, isPending: false }),
}));
vi.mock("@/hooks/use-platform-onboarding", () => ({
  useAdminPlatformTargets: () => ({
    data: { orgs: [{ id: sponsor, display_name: "AI sponsor" }] },
  }),
}));
const unavailable = {
  status: "unavailable" as const,
  reason: "public_not_ready" as const,
  checked_at: null,
  expires_at: null,
  evidence_digest: null,
};
const ready = () => ({
  status: "ready",
  reason: null,
  checked_at: new Date().toISOString(),
  expires_at: new Date(Date.now() + 60000).toISOString(),
  evidence_digest: "server-proof",
});
const profile = {
  id: "profile",
  generation: 7,
  funding_owner_user_id: sponsor,
  guide: null,
  deleted_at: null,
} as PlatformProfile;
beforeEach(() => {
  query.data = {
    contract_version: 1,
    supported: true,
    runtime: null,
    public_readiness: { ...unavailable, reason: "runtime_uninitialized" },
  };
  mutateAsync.mockReset().mockImplementation(async ({ action }) =>
    action === "profiles"
      ? {
          profiles: [{ id: "nyxagent/chat", label: "NyxAgent chat" }],
          default_profile: "nyxagent/chat",
          readiness: unavailable,
        }
      : action === "readiness"
        ? { public_readiness: ready() }
        : query.data,
  );
});

it("does no automatic paid discovery and initializes both lanes with the exact creation CAS", async () => {
  render(<AiRuntimeForm profile={profile} value={query.data} />);
  expect(mutateAsync).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Total public budget per day")).toHaveValue(
    "10",
  );
  expect(screen.getByLabelText("Free quota per person per day")).toHaveValue(
    "0.5",
  );
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(3));
  expect(mutateAsync.mock.calls[2]![0]).toEqual({
    action: "update",
    body: {
      initialize: true,
      expected_profile_generation: 7,
      expected_manager_authority_epoch: 0,
      expected_personal_control_revision: 0,
      expected_public_policy_revision: 0,
      personal_enabled: true,
      public_enabled: true,
      public_policy: {
        source: "nyxagent",
        profile: "nyxagent/chat",
        sponsor_owner_user_id: sponsor,
        total_daily_credits: "10",
        sender_daily_credits: "0.5",
      },
    },
  });
});

it("explains rejected public authority without blaming the selected profile", async () => {
  mutateAsync.mockResolvedValue({
    profiles: [],
    default_profile: null,
    readiness: { ...unavailable, reason: "public_authority_rejected" },
  });
  render(<AiRuntimeForm profile={profile} value={query.data} />);
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  await waitFor(() =>
    expect(screen.getByRole("status")).toHaveTextContent(
      /NyxAgent rejected the public authority assertion/,
    ),
  );
  expect(mutateAsync).toHaveBeenCalledTimes(1);
});

it("gives an actionable sponsor funding error and does not enable after a failed probe", async () => {
  mutateAsync.mockImplementation(async ({ action }) => {
    if (action === "profiles")
      return {
        profiles: [{ id: "public-guide", label: "Public guide" }],
        default_profile: "public-guide",
        readiness: unavailable,
      };
    throw new ApiError(402, {
      error: "insufficient_credits",
      error_code: 11300,
      message: "Insufficient billing credits",
    });
  });
  render(<AiRuntimeForm profile={profile} value={query.data} />);
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  expect(
    await screen.findByText(/Add credits to the sponsor organization/),
  ).toBeInTheDocument();
  expect(mutateAsync).toHaveBeenCalledTimes(2);
});

it("does not enable either lane on unavailable public proof, with explicit personal-only initialization available", async () => {
  mutateAsync.mockImplementation(async ({ action }) =>
    action === "profiles"
      ? {
          profiles: [{ id: "nyxagent/chat", label: "Chat" }],
          default_profile: "nyxagent/chat",
          readiness: unavailable,
        }
      : { public_readiness: unavailable },
  );
  render(<AiRuntimeForm profile={profile} value={query.data} />);
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  await screen.findByText(
    /Public access is unavailable. Settings were not enabled/,
  );
  expect(mutateAsync.mock.calls.map(([x]) => x.action)).toEqual([
    "profiles",
    "readiness",
  ]);
  expect(
    screen.queryByRole("button", { name: "Enable personal access only" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Clear manager emergency stop" }),
  ).not.toBeInTheDocument();
  expect(mutateAsync).toHaveBeenCalledTimes(2);
});

it("retains exact existing money and keeps lane controls out of public AI save", async () => {
  query.data.runtime = {
    manager_authority_epoch: 1,
    personal_control_revision: 2,
    public_policy_revision: 3,
    personal_enabled: false,
    public_enabled: true,
    emergency_stopped: false,
    public_policy: {
      source: "nyxagent",
      profile: "nyxagent/chat",
      sponsor_owner_user_id: sponsor,
      total_daily_credits: "2.000000000001",
      sender_daily_credits: "0.123456789012",
    },
  };
  render(<AiRuntimeForm profile={profile} value={query.data} />);
  expect(screen.getByLabelText("Free quota per person per day")).toHaveValue(
    "0.123456789012",
  );
  fireEvent.change(screen.getByLabelText("Total public budget per day"), {
    target: { value: "3.000000000001" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save public settings" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(3));
  expect(mutateAsync.mock.calls[2]![0].body).toEqual({
    expected_manager_authority_epoch: 1,
    expected_personal_control_revision: 2,
    expected_public_policy_revision: 3,
    public_enabled: true,
    public_policy: {
      source: "nyxagent",
      profile: "nyxagent/chat",
      total_daily_credits: "3.000000000001",
      sender_daily_credits: "0.123456789012",
    },
  });
  expect(
    screen.queryByRole("button", { name: "Pause public guide" }),
  ).not.toBeInTheDocument();
});

it("preserves legacy saved monetary limits and distinguishes unsupported routes", () => {
  const saved = {
    ...profile,
    guide: {
      limits: { daily_credits: "9.123456789012" },
      anonymous_daily_credits: "0.2",
    },
  } as PlatformProfile;
  const view = render(<AiRuntimeForm profile={saved} value={query.data} />);
  expect(screen.getByLabelText("Total public budget per day")).toHaveValue(
    "9.123456789012",
  );
  query.data = { ...query.data, supported: false };
  view.rerender(<ManagerRuntimeSettings profile={saved} />);
  expect(screen.getByText(/not supported by this server/)).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Enable bot" }),
  ).not.toBeInTheDocument();
});

it("reports an unsupported public runtime without blaming the selected profile or enabling anything", async () => {
  mutateAsync.mockResolvedValue({
    profiles: [],
    default_profile: null,
    readiness: { ...unavailable, reason: "public_contract_unsupported" },
  });
  render(<AiRuntimeForm profile={profile} value={query.data} />);
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  expect(
    (
      await screen.findAllByText(
        /Upgrade it to a version with the public runtime contract/,
      )
    ).length,
  ).toBeGreaterThan(0);
  expect(
    screen.queryByText(/requested public profile is unavailable/i),
  ).not.toBeInTheDocument();
  expect(mutateAsync.mock.calls.map(([x]) => x.action)).toEqual(["profiles"]);
});

it("requires an explicit selection when discovery advertises no public default", async () => {
  mutateAsync.mockResolvedValue({
    profiles: [{ id: "public/guide", label: "Guide" }],
    default_profile: null,
    readiness: unavailable,
  });
  render(<AiRuntimeForm profile={profile} value={query.data} />);
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  expect(
    await screen.findByText(/This runtime has no public default/),
  ).toBeInTheDocument();
  expect(mutateAsync.mock.calls.map(([x]) => x.action)).toEqual(["profiles"]);
});

it("uses the advertised public default for first enable and omits a guessed profile from discovery", async () => {
  mutateAsync.mockImplementation(async ({ action }) =>
    action === "profiles"
      ? {
          profiles: [{ id: "public/guide", label: "Public guide" }],
          default_profile: "public/guide",
          readiness: unavailable,
        }
      : action === "readiness"
        ? { public_readiness: ready() }
        : query.data,
  );
  render(<AiRuntimeForm profile={profile} value={query.data} />);
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(3));
  expect(
    mutateAsync.mock.calls[0]![0].body.public_policy.profile,
  ).toBeUndefined();
  expect(mutateAsync.mock.calls[1]![0].body.public_policy.profile).toBe(
    "public/guide",
  );
  expect(mutateAsync.mock.calls[2]![0].body.public_policy.profile).toBe(
    "public/guide",
  );
});

it("rejects expired readiness without sending an update", async () => {
  mutateAsync.mockImplementation(async ({ action }) =>
    action === "profiles"
      ? {
          profiles: [{ id: "public/guide", label: "Public guide" }],
          default_profile: "public/guide",
          readiness: unavailable,
        }
      : {
          public_readiness: {
            ...ready(),
            expires_at: new Date(Date.now() - 1000).toISOString(),
            reason: "proof_expired",
          },
        },
  );
  render(<AiRuntimeForm profile={profile} value={query.data} />);
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  expect(
    await screen.findByText(/Settings were not enabled/),
  ).toBeInTheDocument();
  expect(mutateAsync.mock.calls.map(([x]) => x.action)).toEqual([
    "profiles",
    "readiness",
  ]);
});

it("allows applying a changed deployment with unchanged public settings and binds the validated digest", async () => {
  query.data.runtime = {
    manager_authority_epoch: 1,
    personal_control_revision: 2,
    public_policy_revision: 3,
    personal_enabled: false,
    public_enabled: true,
    emergency_stopped: false,
    activated_policy_digest: "a".repeat(64),
    public_policy: {
      source: "nyxagent",
      profile: "nyxagent/chat",
      sponsor_owner_user_id: sponsor,
      total_daily_credits: "10",
      sender_daily_credits: "0.5",
    },
  };
  mutateAsync.mockImplementation(async ({ action }) =>
    action === "profiles"
      ? {
          profiles: [{ id: "nyxagent/chat", label: "Public guide" }],
          default_profile: "nyxagent/chat",
          readiness: unavailable,
        }
      : action === "readiness"
        ? { public_readiness: { ...ready(), policy_digest: "b".repeat(64) } }
        : query.data,
  );
  render(<AiRuntimeForm profile={profile} value={query.data} />);
  const apply = screen.getByRole("button", {
    name: "Validate and apply public deployment",
  });
  expect(apply).toBeEnabled();
  fireEvent.click(apply);
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(3));
  expect(mutateAsync.mock.calls[2]![0].body).toMatchObject({
    public_enabled: true,
    public_deployment_digest: "b".repeat(64),
    expected_public_policy_revision: 3,
  });
  expect(mutateAsync.mock.calls[2]![0].body).not.toHaveProperty(
    "personal_enabled",
  );
  expect(screen.getByRole("status")).toHaveTextContent(
    "A changed deployment ends existing public sessions so senders can start fresh",
  );
});
