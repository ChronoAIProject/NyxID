import { act, render, screen, waitFor } from "@testing-library/react";
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
  useAuthStore.setState({ user: { id: "person" } as User });
});
it("limits ranking to 200 without disabling unranked service controls", async () => {
  const rows = inventory(201);
  render(
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
  await userEvent.click(
    screen.getByRole("button", { name: "Rank Service 200" }),
  );
  expect(screen.getByText(/You can rank at most 200/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  await userEvent.click(
    screen.getByRole("button", { name: "Unrank Service 0" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Rank Service 200" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalled());
  expect(mutateAsync.mock.calls[0]?.[0].ordered).toHaveLength(200);
  expect(mutateAsync.mock.calls[0]?.[0].ordered).not.toContain(rows[0]?.id);
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
