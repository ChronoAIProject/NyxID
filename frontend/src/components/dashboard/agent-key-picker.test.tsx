import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { accountResourcePreferenceKey } from "@/lib/account-resource-preferences";
import { AgentKeyPicker } from "./agent-key-picker";

const keys = [
  { id: "first", name: "First", key_prefix: "preview-first", is_active: true },
  { id: "second", name: "Second", key_prefix: "preview-second", is_active: true },
  { id: "inactive", name: "Inactive", key_prefix: "preview-inactive", is_active: false },
];
vi.mock("@/hooks/use-api-keys", () => ({ useApiKeys: () => ({ data: keys }) }));
const key = () => accountResourcePreferenceKey("nyxid:last-picked-agent-key");
beforeEach(() => { localStorage.clear(); transitionAssistantIdentity("A"); });
it("persists only an explicit pick under the current owner's key", async () => {
  const selected = vi.fn();
  const writes = vi.spyOn(localStorage, "setItem");
  render(<AgentKeyPicker onSelect={selected} />);
  expect(selected).toHaveBeenLastCalledWith(expect.objectContaining({ id: "first" }));
  expect(writes).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("combobox"));
  await userEvent.click(screen.getByRole("option", { name: "Second" }));
  await waitFor(() => expect(selected).toHaveBeenLastCalledWith(expect.objectContaining({ id: "second" })));
  expect(writes).toHaveBeenCalledExactlyOnceWith(key(), "second");
});
it.each([["second", "second"], ["inactive", "first"], ["missing", "first"]])("restores %s only when still active", (stored, expected) => {
  localStorage.setItem(key(), stored);
  const selected = vi.fn();
  const writes = vi.spyOn(localStorage, "setItem");
  writes.mockClear();
  render(<AgentKeyPicker onSelect={selected} />);
  expect(selected).toHaveBeenLastCalledWith(expect.objectContaining({ id: expected }));
  expect(writes).not.toHaveBeenCalled();
});
it("never restores another owner's pick", () => {
  localStorage.setItem(key(), "second");
  act(() => transitionAssistantIdentity("B"));
  const selected = vi.fn();
  render(<AgentKeyPicker onSelect={selected} />);
  expect(selected).toHaveBeenLastCalledWith(expect.objectContaining({ id: "first" }));
  expect(localStorage.getItem(key())).toBeNull();
});
