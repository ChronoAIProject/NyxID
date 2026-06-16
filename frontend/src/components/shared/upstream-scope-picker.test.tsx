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
  lockedScopes,
  grantedScopes,
  providerName,
  onChangeSpy,
}: {
  catalog?: ScopeCatalogEntry[];
  defaultScopes?: string[];
  initial?: string[];
  lockedScopes?: string[];
  grantedScopes?: string[];
  providerName?: string;
  onChangeSpy?: (s: readonly string[]) => void;
}) {
  const [value, setValue] = useState<readonly string[]>(initial);
  return (
    <UpstreamScopePicker
      catalog={catalog}
      defaultScopes={defaultScopes}
      value={value}
      lockedScopes={lockedScopes}
      grantedScopes={grantedScopes}
      providerName={providerName}
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

  it("renders a legend and sr-only text for sensitive scopes (not color-only)", () => {
    render(<Harness />);
    // Legend explaining the warning dot is present because media.write is sensitive.
    expect(screen.getByText(/write or admin-level scope/i)).toBeInTheDocument();
    // The sensitive pill carries non-visual meaning for screen readers.
    const uploadMedia = screen.getByRole("button", { name: /Upload media/i });
    expect(uploadMedia).toHaveTextContent(/write or admin access/i);
  });

  it("omits the sensitive legend when no scope is sensitive", () => {
    render(
      <Harness
        catalog={[{ scope: "read", label: "Read", description: "Read." }]}
        defaultScopes={["read"]}
        initial={["read"]}
      />,
    );
    expect(screen.queryByText(/write or admin-level scope/i)).not.toBeInTheDocument();
  });

  // Append-only edit mode (NyxID#917 follow-up): granted scopes are locked.
  it("renders locked scopes as selected, non-removable, tagged 'granted'", () => {
    render(
      <Harness
        defaultScopes={[]}
        initial={["tweet.read"]}
        lockedScopes={["tweet.read"]}
      />,
    );
    const granted = screen.getByRole("button", { name: /Read posts/i });
    expect(granted).toHaveAttribute("aria-pressed", "true");
    expect(granted).toBeDisabled();
    expect(granted).toHaveTextContent(/granted/i);
  });

  it("does not remove a locked scope when clicked", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(
      <Harness
        defaultScopes={[]}
        initial={["tweet.read"]}
        lockedScopes={["tweet.read"]}
        onChangeSpy={onChange}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Read posts/i }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it("allows adding a scope alongside locked ones (append-only)", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(
      <Harness
        defaultScopes={[]}
        initial={["tweet.read"]}
        lockedScopes={["tweet.read"]}
        onChangeSpy={onChange}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Upload media/i }));
    expect(onChange).toHaveBeenLastCalledWith(
      expect.arrayContaining(["tweet.read", "media.write"]),
    );
  });

  it("shows the locked helper note when scopes can't be removed", () => {
    render(
      <Harness defaultScopes={[]} initial={["tweet.read"]} lockedScopes={["tweet.read"]} />,
    );
    expect(screen.getByText(/already authorized and locked/i)).toBeInTheDocument();
    expect(screen.getByText(/can.t be removed here/i)).toBeInTheDocument();
  });

  // Declarative edit mode (NyxID#917 follow-up): grantedScopes drives a
  // change summary; removing a granted scope shows a warning.
  it("shows a change summary when adding a scope in edit mode", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        defaultScopes={[]}
        initial={["tweet.read"]}
        grantedScopes={["tweet.read"]}
      />,
    );
    // Nothing changed yet → no summary.
    expect(screen.queryByText(/Adding:/i)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Upload media/i }));
    const addingLine = screen.getByText(/Adding:/i).closest("p");
    expect(addingLine).toHaveTextContent(/Upload media/);
  });

  it("shows a removal warning when a granted scope is deselected in edit mode", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        defaultScopes={[]}
        initial={["tweet.read", "media.write"]}
        grantedScopes={["tweet.read", "media.write"]}
        providerName="Twitter / X"
      />,
    );
    await user.click(screen.getByRole("button", { name: /Upload media/i }));
    expect(screen.getByText(/Removing:/i)).toBeInTheDocument();
    expect(screen.getByText(/stop any app that relies on it/i)).toBeInTheDocument();
    expect(screen.getByText(/Twitter \/ X/)).toBeInTheDocument();
  });

  it("allows deselecting a granted scope when it is not locked", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(
      <Harness
        defaultScopes={[]}
        initial={["tweet.read", "media.write"]}
        grantedScopes={["tweet.read", "media.write"]}
        onChangeSpy={onChange}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Upload media/i }));
    expect(onChange).toHaveBeenLastCalledWith(["tweet.read"]);
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
