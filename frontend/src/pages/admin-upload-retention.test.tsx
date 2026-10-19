import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { AdminUploadRetentionPage } from "./admin-upload-retention";
import {
  uploadRetentionPolicySchema,
  type UploadRetentionResponse,
} from "@/schemas/upload-retention";
const { query, mutate } = vi.hoisted(() => ({
  query: vi.fn(),
  mutate: vi.fn(),
}));
vi.mock("@/hooks/use-upload-retention", () => ({
  useUploadRetention: query,
  useUpdateUploadRetention: () => ({ mutateAsync: mutate, isPending: false }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
const defaults = {
  pending_hours: 24,
  image_days: 30,
  images_delete_after_turn: false,
  document_days: 30,
  tool_image_days: null,
};
const data: UploadRetentionResponse = {
  effective: defaults,
  defaults,
  overridden: true,
  revision: 2,
  updated_at: null,
  replica_revision: 1,
  replica_effective: defaults,
  replica_refreshed_at: "2026-10-03T00:00:00Z",
  refresh_seconds: 5,
};
beforeEach(() => {
  vi.clearAllMocks();
  query.mockReturnValue({ data, isPending: false, isError: false });
  mutate.mockImplementation(async (value) => ({
    ...data,
    effective: value ?? defaults,
  }));
});
it("shows current values, defaults, replica refresh and the upstream-copy notice", () => {
  render(<AdminUploadRetentionPage />);
  expect(screen.getByLabelText("Unsent uploads (hours)")).toHaveValue(24);
  expect(screen.getByLabelText("Sent documents (days)")).toHaveValue(30);
  expect(screen.getByText(/NyxID’s copy only/)).toBeInTheDocument();
  expect(screen.getByText(/Images an agent already saw/)).toHaveTextContent(
    "Model providers",
  );
  expect(screen.getByText(/refresh pending/)).toHaveTextContent("revision 2");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});
it("dirty-gates Save and persists the after-turn switch and numeric values", async () => {
  render(<AdminUploadRetentionPage />);
  fireEvent.change(screen.getByLabelText("Unsent uploads (hours)"), {
    target: { value: "2" },
  });
  fireEvent.click(
    screen.getByRole("switch", {
      name: "Delete images after their first turn",
    }),
  );
  const save = screen.getByRole("button", { name: "Save" });
  await waitFor(() => expect(save).toBeEnabled());
  fireEvent.click(save);
  await waitFor(() =>
    expect(mutate).toHaveBeenCalledWith({
      ...defaults,
      pending_hours: 2,
      images_delete_after_turn: true,
    }),
  );
});
it("switches tool images between bounded days and keep-with-conversation", async () => {
  render(<AdminUploadRetentionPage />);
  fireEvent.click(
    screen.getByRole("switch", {
      name: "Keep tool images with the conversation",
    }),
  );
  fireEvent.change(screen.getByLabelText("Tool images (days)"), {
    target: { value: "7" },
  });
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(mutate).toHaveBeenCalledWith({ ...defaults, tool_image_days: 7 }),
  );
});
it("rejects out-of-bounds edits and restores defaults through DELETE", async () => {
  render(<AdminUploadRetentionPage />);
  fireEvent.change(screen.getByLabelText("Sent documents (days)"), {
    target: { value: "366" },
  });
  await waitFor(() =>
    expect(screen.getByRole("alert")).toHaveTextContent("365"),
  );
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Restore defaults" }));
  await waitFor(() => expect(mutate).toHaveBeenCalledWith(null));
});
it("does not overwrite unsaved edits when the replica refreshes", async () => {
  const { rerender } = render(<AdminUploadRetentionPage />);
  fireEvent.change(screen.getByLabelText("Unsent uploads (hours)"), {
    target: { value: "9" },
  });
  query.mockReturnValue({
    data: {
      ...data,
      revision: 3,
      effective: { ...defaults, pending_hours: 48 },
    },
    isPending: false,
    isError: false,
  });
  rerender(<AdminUploadRetentionPage />);
  expect(screen.getByLabelText("Unsent uploads (hours)")).toHaveValue(9);
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.getByLabelText("Unsent uploads (hours)")).toHaveValue(48);
});
it("reports loading and fetch failure without presenting defaults as effective", () => {
  query.mockReturnValue({ isPending: true });
  const { rerender } = render(<AdminUploadRetentionPage />);
  expect(screen.getByRole("status")).toHaveTextContent("Loading");
  query.mockReturnValue({ isError: true });
  rerender(<AdminUploadRetentionPage />);
  expect(
    screen.getByText("Could not load retention policy."),
  ).toBeInTheDocument();
  expect(
    screen.queryByLabelText("Unsent uploads (hours)"),
  ).not.toBeInTheDocument();
});
it("enforces the server's numeric bounds in the schema", () => {
  expect(
    uploadRetentionPolicySchema.safeParse({ ...defaults, pending_hours: 8760 })
      .success,
  ).toBe(true);
  for (const bad of [
    { pending_hours: 0 },
    { pending_hours: 8761 },
    { image_days: 366 },
    { document_days: 0 },
    { tool_image_days: 366 },
  ]) {
    expect(
      uploadRetentionPolicySchema.safeParse({ ...defaults, ...bad }).success,
    ).toBe(false);
  }
});
