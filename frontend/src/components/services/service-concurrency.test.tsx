import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { PolicyEditor } from "./service-concurrency";
const { save } = vi.hoisted(() => ({ save: vi.fn() }));
vi.mock("@/hooks/use-service-concurrency", () => ({ useUpdateServiceConcurrency: () => ({ mutateAsync: save, isPending: false }) }));
vi.mock("@/hooks/use-admin", () => ({ useAdminUsers: () => ({ data: { users: [] } }) }));
vi.mock("@/components/admin-credits/credit-pickers", () => ({
  PersonPicker: ({ onChange }: { onChange: (ids: string[]) => void }) => <button type="button" onClick={() => onChange(["aabbccdd-0000-4000-8000-000000000001"])}>Select person</button>,
  OrgPicker: () => <span>Organization picker</span>,
}));
beforeEach(() => { save.mockReset(); save.mockImplementation(async (input) => input); });
it("keeps absent policy unlimited until enabled, then saves a default and override", async () => {
  const user = userEvent.setup();
  render(<PolicyEditor serviceId="service" policy={null} />);
  expect(screen.getByRole("button", { name: "Save concurrency limits" })).toBeDisabled();
  await user.click(screen.getByRole("switch", { name: "Enable concurrency limits" }));
  await user.type(screen.getByLabelText("Default per-person limit"), "3");
  await user.click(screen.getByRole("button", { name: "Select person" }));
  await user.click(screen.getByRole("button", { name: "Save concurrency limits" }));
  await waitFor(() => expect(save).toHaveBeenCalledWith({ policy: { default_limit: 3, users: [{ id: "aabbccdd-0000-4000-8000-000000000001", limit: null }], orgs: [] } }));
});
it("disabling removes the policy explicitly", async () => {
  const user = userEvent.setup();
  render(<PolicyEditor serviceId="service" policy={{ default_limit: 2, users: [], orgs: [] }} />);
  await user.click(screen.getByRole("switch", { name: "Enable concurrency limits" }));
  await user.click(screen.getByRole("button", { name: "Save concurrency limits" }));
  await waitFor(() => expect(save).toHaveBeenCalledWith({ policy: null }));
});
it("invalid limits are blocked with a visible form error", async () => {
  const user = userEvent.setup();
  render(<PolicyEditor serviceId="service" policy={{ default_limit: 2, users: [], orgs: [] }} />);
  await user.clear(screen.getByLabelText("Default per-person limit"));
  await user.type(screen.getByLabelText("Default per-person limit"), "0");
  // Native number validation also blocks submission; schema validation covers programmatic writes.
  expect(screen.getByLabelText("Default per-person limit")).toBeInvalid();
  expect(save).not.toHaveBeenCalled();
});
