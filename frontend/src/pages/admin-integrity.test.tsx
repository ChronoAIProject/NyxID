import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { toast } from "sonner";
import { AdminIntegrityPage } from "./admin-integrity";
import { chainVerificationResponseSchema } from "@/schemas/admin-chain-verification";

const { query, mutate } = vi.hoisted(() => ({
  query: vi.fn(),
  mutate: vi.fn(),
}));
vi.mock("@/hooks/use-admin-chain-verification", () => ({
  useChainVerification: query,
  useRunChainVerification: () => ({ mutate, isPending: false }),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (state: unknown) => unknown) =>
    select({
      user: { id: "admin", is_admin: true, role: "admin" },
    }),
}));
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), warning: vi.fn(), error: vi.fn() },
}));

const localStatus = {
  chain: "audit_log",
  outcome: "ok",
  cursor_seq: 4,
  head_seq: 3,
  checked_entries: 3,
  last_run_at: "2026-10-20T00:00:00Z",
};

function response(status?: string) {
  return chainVerificationResponseSchema.parse({
    chains: [localStatus],
    ...(status
      ? {
          external_evidence: {
            status,
            local_head_seq: 3,
            witnessed_seq: status === "unwitnessed" ? null : 2,
            pending_count: 1,
          },
        }
      : {}),
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  query.mockReturnValue({ data: response("unwitnessed"), isLoading: false });
});

it("shows unwitnessed evidence separately from local continuity", () => {
  render(<AdminIntegrityPage />);
  expect(screen.getByText("Unwitnessed")).toBeVisible();
  expect(screen.getByText("Locally intact")).toBeVisible();
  expect(
    screen.getByText(/cannot detect a coordinated database rollback/),
  ).toBeVisible();
  expect(screen.queryByText(/^Intact$/)).not.toBeInTheDocument();
});

it("does not treat a stored witness cursor as fresh independent proof", () => {
  query.mockReturnValue({ data: response("unverified") });
  render(<AdminIntegrityPage />);
  expect(screen.getByText("Unverified")).toBeVisible();
  expect(screen.getByText(/stored acknowledgment alone/)).toBeVisible();
});

it("treats an older replica without an evidence response as unavailable", () => {
  query.mockReturnValue({ data: response() });
  render(<AdminIntegrityPage />);
  expect(screen.getByText("Unavailable")).toBeVisible();
  expect(
    screen.getByText(/cannot establish protection against database rollback/),
  ).toBeVisible();
});

it("shows independent conflict even when local links pass", () => {
  query.mockReturnValue({ data: response("broken") });
  render(<AdminIntegrityPage />);
  expect(screen.getByText("Conflicting history")).toBeVisible();
  expect(screen.getByText("Locally intact")).toBeVisible();
});

it("never announces verified success for an unwitnessed local check", () => {
  mutate.mockImplementation((_input, options) =>
    options.onSuccess(response("unwitnessed")),
  );
  render(<AdminIntegrityPage />);
  fireEvent.click(screen.getByRole("button", { name: "Run check now" }));
  expect(toast.success).not.toHaveBeenCalled();
  expect(toast.warning).toHaveBeenCalledWith(
    "Local check completed; independent evidence: unwitnessed",
  );
});

it("alarms on an external conflict from a manual check", () => {
  mutate.mockImplementation((_input, options) =>
    options.onSuccess(response("broken")),
  );
  render(<AdminIntegrityPage />);
  fireEvent.click(screen.getByRole("button", { name: "Run check now" }));
  expect(toast.error).toHaveBeenCalledWith(
    "Independent evidence conflicts with stored history",
  );
  expect(toast.success).not.toHaveBeenCalled();
});

it("shows unknown restart freshness, per-tier admission, limits and closure times", () => {
  const data = response("unverified");
  query.mockReturnValue({
    data: chainVerificationResponseSchema.parse({
      ...data,
      external_evidence: {
        ...data.external_evidence,
        diagnostics: {
          witness: "backlogged",
          admission: {
            narrowing: "alarm",
            availability: "alarm",
            standard: "closed",
            effect: "closed",
          },
          chain_id: "6d53db16-51f6-491c-ad87-39f27e413e75",
          pending_count: 1,
          oldest_pending_secs: 3600,
          last_contact_secs: null,
          local_head_seq: 3,
          witnessed_seq: 2,
          standard_closes_in_secs: 0,
          availability_closes_in_secs: 82800,
          divergence: null,
          limits: {
            contact_secs: 60,
            pending_age_secs: 30,
            pending_count: 10000,
            standard: { pending_age_secs: 3600, pending_count: 100000 },
            availability: { pending_age_secs: 86400, pending_count: 1000000 },
          },
        },
      },
    }),
  });
  render(<AdminIntegrityPage />);
  expect(screen.getByText("Audit witness")).toBeVisible();
  expect(
    screen.getByText(/Contact freshness unknown since process start/),
  ).toBeVisible();
  expect(screen.getByText("Standard time to closure")).toBeVisible();
  expect(screen.getByText("0 s")).toBeVisible();
  expect(screen.getByLabelText("Audit admission by tier")).toHaveTextContent(
    "availabilityalarm",
  );
  expect(screen.getByLabelText("Audit admission by tier")).toHaveTextContent(
    "standardclosed",
  );
  expect(
    screen.getByRole("link", { name: "Witness recovery runbook" }),
  ).toBeVisible();
});
