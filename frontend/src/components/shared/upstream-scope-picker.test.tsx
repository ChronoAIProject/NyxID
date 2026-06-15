import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { UpstreamScopePicker } from "./upstream-scope-picker";
import type { ScopeCatalogEntry } from "@/types/keys";

const CATALOG: ScopeCatalogEntry[] = [
  { scope: "tweet.read", label: "Read posts", description: "Read posts." },
  { scope: "media.write", label: "Upload media", description: "Upload media.", sensitive: true },
];

/** Controlled harness — mirrors how the dialogs own the selection state. */
function Harness({
  catalog = CATALOG,
  defaultScopes = ["tweet.read"],
  initial = ["tweet.read"],
  onChangeSpy,
}: {
  catalog?: ScopeCatalogEntry[];
  defaultScopes?: string[];
  initial?: string[];
  onChangeSpy?: (s: readonly string[]) => void;
}) {
  const [value, setValue] = useState<readonly string[]>(initial);
  return (
    <UpstreamScopePicker
      catalog={catalog}
      defaultScopes={defaultScopes}
      value={value}
      onChange={(next) => {
        onChangeSpy?.(next);
        setValue(next);
      }}
    />
  );
}

describe("UpstreamScopePicker", () => {
  it("renders catalog scopes as pills, defaults marked and pre-selected", () => {
    render(<Harness />);
    const readPosts = screen.getByRole("button", { name: /Read posts/i });
    const uploadMedia = screen.getByRole("button", { name: /Upload media/i });
    // Default scope is pre-selected (aria-pressed); non-default is not.
    expect(readPosts).toHaveAttribute("aria-pressed", "true");
    expect(uploadMedia).toHaveAttribute("aria-pressed", "false");
    // Default marker is shown on the default pill.
    expect(readPosts).toHaveTextContent(/default/i);
  });

  it("toggling a pill on adds it to the selection", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Harness onChangeSpy={onChange} />);
    await user.click(screen.getByRole("button", { name: /Upload media/i }));
    expect(onChange).toHaveBeenLastCalledWith(
      expect.arrayContaining(["tweet.read", "media.write"]),
    );
  });

  it("toggling a default pill off removes it (defaults are removable)", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Harness onChangeSpy={onChange} />);
    await user.click(screen.getByRole("button", { name: /Read posts/i }));
    expect(onChange).toHaveBeenLastCalledWith([]);
  });

  it("adds a custom scope via the Add button, deduped", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Harness onChangeSpy={onChange} />);
    const input = screen.getByPlaceholderText(/custom\.scope/i);
    await user.type(input, "dm.read, dm.read");
    await user.click(screen.getByRole("button", { name: /^Add$/i }));
    expect(onChange).toHaveBeenLastCalledWith(["tweet.read", "dm.read"]);
  });

  it("custom-added scope renders as a removable selected pill", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const input = screen.getByPlaceholderText(/custom\.scope/i);
    await user.type(input, "dm.read");
    await user.click(screen.getByRole("button", { name: /^Add$/i }));
    const customPill = await screen.findByRole("button", { name: /dm\.read/i });
    expect(customPill).toHaveAttribute("aria-pressed", "true");
  });

  it("Add is disabled for empty/whitespace input", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const add = screen.getByRole("button", { name: /^Add$/i });
    expect(add).toBeDisabled();
    await user.type(screen.getByPlaceholderText(/custom\.scope/i), "   ");
    expect(add).toBeDisabled();
  });

  it("shows default scopes as pills even when absent from the catalog", () => {
    render(
      <Harness catalog={[]} defaultScopes={["offline_access"]} initial={["offline_access"]} />,
    );
    const pill = screen.getByRole("button", { name: /offline_access/i });
    expect(pill).toHaveAttribute("aria-pressed", "true");
    expect(pill).toHaveTextContent(/default/i);
  });
});
