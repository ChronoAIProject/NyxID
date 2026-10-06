import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { ServicePreferenceEditor } from "./service-preference-editor";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type { KeyInfo } from "@/types/keys";
import { ApiError } from "@/lib/api-client";
const { mutateAsync } = vi.hoisted(() => ({ mutateAsync: vi.fn() }));
vi.mock("@/hooks/use-service-preference", () => ({
  useSaveServicePreference: () => ({ mutateAsync, isPending: false }),
}));
function inventory(count: number) {
  return Array.from(
    { length: count },
    (_, i) =>
      ({
        id: `${String(i).padStart(8, "0")}-1111-4111-8111-111111111111`,
        label: `Service ${i}`,
        credential_source: { type: "personal" },
      }) as KeyInfo,
  );
}
beforeEach(() => {
  vi.clearAllMocks();
  mutateAsync.mockReset().mockResolvedValue(undefined);
  useAuthStore.setState({ user: { id: "person" } as User });
});
it("limits ranking to 200 without disabling unranked service controls", async () => {
  const rows = inventory(201);
  const { container } = render(
    <ServicePreferenceEditor
      preference={{
        ordered: rows.slice(0, 200).map((row) => row.id),
        version: 1,
        updated_at: null,
      }}
      inventory={rows}
      viewMode="table"
      blocked={false}
      onClose={vi.fn()}
      onDirtyChange={vi.fn()}
      reloadPreference={vi.fn()}
      refreshInventory={vi.fn()}
    />,
  );
  const user = userEvent.setup();
  const row = (id: string) =>
    within(
      container.querySelector<HTMLElement>(`[data-preference-item="${id}"]`)!,
    );
  const footer = within(
    container.querySelector("form")!.lastElementChild as HTMLElement,
  );
  const save = footer.getByRole("button", { name: "Save" });
  await user.click(
    row(rows[200]!.id).getByRole("button", { name: "Rank Service 200" }),
  );
  expect(container.querySelector('form > [role="status"]')).toHaveTextContent(
    "You can rank at most 200",
  );
  expect(save).toBeDisabled();
  expect(mutateAsync).not.toHaveBeenCalled();
  await user.click(
    row(rows[0]!.id).getByRole("button", { name: "Unrank Service 0" }),
  );
  await user.click(
    row(rows[200]!.id).getByRole("button", { name: "Rank Service 200" }),
  );
  expect(save).toBeEnabled();
  await user.click(save);
  await waitFor(() =>
    expect(mutateAsync).toHaveBeenCalledExactlyOnceWith({
      ordered: rows.slice(1).map((row) => row.id),
      expected_version: 1,
    }),
  );
});
it("stale recovery only prunes after a successful fresh inventory read", async () => {
  const rows = inventory(2);
  const refresh = vi
    .fn()
    .mockRejectedValueOnce(new TypeError("offline"))
    .mockResolvedValueOnce([rows[0]]);
  mutateAsync.mockRejectedValue(
    new ApiError(400, {
      error: "validation",
      message: "unknown service id",
      error_code: 1002,
    }),
  );
  render(
    <ServicePreferenceEditor
      preference={{ ordered: [rows[0]!.id], version: 1, updated_at: null }}
      inventory={rows}
      viewMode="grid"
      blocked={false}
      onClose={vi.fn()}
      onDirtyChange={vi.fn()}
      reloadPreference={vi.fn()}
      refreshInventory={refresh}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Rank Service 1" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await userEvent.click(await screen.findByRole("button", { name: "Retry" }));
  await screen.findByText(/Could not refresh services/);
  expect(screen.getByText("Service 1")).toBeInTheDocument();
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 1500));
  });
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() =>
    expect(screen.queryByText("Service 1")).not.toBeInTheDocument(),
  );
});

it("enriches late and refreshed org provenance without replacing the draft or version", async () => {
  const rows = inventory(2).map((row) => ({
    ...row,
    credential_source: undefined,
  }));
  const org = {
    type: "org" as const,
    org_id: "team",
    org_name: "Research team",
    role: "viewer" as const,
    allowed: false,
  };
  let sourcesReady = false;
  const enrich = (rows: readonly KeyInfo[]) =>
    rows.map((row) => ({
      ...row,
      credential_source:
        row.credential_source ?? (sourcesReady ? org : undefined),
    }));
  const refresh = vi.fn(async () => enrich(rows));
  const props = {
    preference: { ordered: [rows[0]!.id], version: 7, updated_at: null },
    inventory: rows,
    enrichInventory: enrich,
    viewMode: "table" as const,
    blocked: false,
    onClose: vi.fn(),
    onDirtyChange: vi.fn(),
    reloadPreference: vi.fn(),
    refreshInventory: refresh,
  };
  const { rerender } = render(<ServicePreferenceEditor {...props} />);
  await userEvent.click(screen.getByRole("button", { name: "Rank Service 1" }));
  sourcesReady = true;
  rerender(<ServicePreferenceEditor {...props} />);
  expect(screen.getAllByText("Org: Research team")).toHaveLength(2);
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  mutateAsync.mockRejectedValueOnce(
    new ApiError(400, {
      error: "validation",
      message: "unknown service id",
      error_code: 1002,
    }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await userEvent.click(await screen.findByRole("button", { name: "Retry" }));
  await waitFor(() => expect(refresh).toHaveBeenCalledOnce());
  expect(screen.getAllByText("Org: Research team")).toHaveLength(2);
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(2));
  expect(mutateAsync).toHaveBeenLastCalledWith({
    ordered: rows.map((row) => row.id),
    expected_version: 7,
  });
  expect(props.onClose).toHaveBeenCalledOnce();
});
