import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

// Issue #787: the DisplayOnce panels are the wizard's one-time-secret
// surface (api-key create/rotate, node register/rotate tokens, MFA
// recovery codes). These tests pin the security-relevant contract:
//   - the secret is masked by default and only un-masked on explicit
//     Reveal (and re-masks on Hide) — the one-time-reveal gate;
//   - copy-to-clipboard writes the *real* secret (never the mask) and
//     surfaces a "copied" confirmation;
//   - the optional secondary secret only renders when supplied;
//   - the acknowledge button drives the pairing-complete callback and
//     locks out while that POST is in flight.

// `userEvent.setup()` installs its own clipboard stub on `navigator`, so a
// module-level Object.defineProperty stub gets clobbered. Instead, spy on
// whatever clipboard is live at click time (after setup) via this helper.
function spyClipboard() {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText },
    configurable: true,
    writable: true,
  });
  return writeText;
}

import { DisplayOncePanel, RecoveryCodesPanel } from "./display-once-panel";

const SECRET = "nyx_nauth_super_secret_value_123";

function renderPanel(overrides: Partial<Parameters<typeof DisplayOncePanel>[0]> = {}) {
  const onAcknowledge = vi.fn();
  render(
    <DisplayOncePanel
      title="Your node auth token"
      description="Paste this into the node config."
      secret={SECRET}
      ackButtonLabel="I saved it — finish"
      onAcknowledge={onAcknowledge}
      isAcknowledging={false}
      {...overrides}
    />,
  );
  return { onAcknowledge };
}

describe("DisplayOncePanel — one-time reveal gate", () => {
  it("masks the secret by default and never renders the raw value up front", () => {
    renderPanel();

    // The header tells the user this is shown once.
    expect(screen.getByText("Shown once — save it now")).toBeInTheDocument();
    expect(screen.getByText("Your node auth token")).toBeInTheDocument();
    // Raw secret is not in the DOM until the user reveals it.
    expect(screen.queryByText(SECRET)).not.toBeInTheDocument();
    // It is masked with bullets instead.
    expect(screen.getByText("•".repeat(SECRET.length))).toBeInTheDocument();
  });

  it("reveals the secret on Reveal and re-masks it on Hide", async () => {
    const user = userEvent.setup();
    renderPanel();

    await user.click(screen.getByRole("button", { name: "Reveal" }));
    expect(screen.getByText(SECRET)).toBeInTheDocument();
    expect(
      screen.queryByText("•".repeat(SECRET.length)),
    ).not.toBeInTheDocument();

    // The same control now toggles back to Hide and re-masks.
    await user.click(screen.getByRole("button", { name: "Hide" }));
    expect(screen.queryByText(SECRET)).not.toBeInTheDocument();
    expect(screen.getByText("•".repeat(SECRET.length))).toBeInTheDocument();
  });
});

describe("DisplayOncePanel — copy to clipboard", () => {
  it("copies the real secret (not the mask) and confirms with a check", async () => {
    const user = userEvent.setup();
    const writeText = spyClipboard();
    renderPanel();

    const copyButton = screen.getByRole("button", { name: "Copy to clipboard" });
    await user.click(copyButton);

    expect(writeText).toHaveBeenCalledWith(SECRET);
    // The icon swaps to the success state; aria-label no longer offered
    // means the green check is showing. Assert the button is still
    // present and the copy fired exactly once with the secret.
    expect(writeText).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(copyButton.querySelector(".text-green-600")).not.toBeNull();
    });
  });
});

describe("DisplayOncePanel — secondary secret + acknowledge", () => {
  it("renders the secondary secret field only when provided", () => {
    const { rerender } = render(
      <DisplayOncePanel
        title="t"
        description="d"
        secret={SECRET}
        ackButtonLabel="ack"
        onAcknowledge={vi.fn()}
        isAcknowledging={false}
      />,
    );
    // Two SecretFields => two reveal toggles when a secondary is present.
    expect(screen.getAllByRole("button", { name: "Reveal" })).toHaveLength(1);

    rerender(
      <DisplayOncePanel
        title="t"
        description="d"
        secret={SECRET}
        secondarySecret={{ label: "Signing key", value: "sk_secondary_999" }}
        ackButtonLabel="ack"
        onAcknowledge={vi.fn()}
        isAcknowledging={false}
      />,
    );
    expect(screen.getByText("Signing key")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Reveal" })).toHaveLength(2);
  });

  it("fires onAcknowledge when the ack button is clicked", async () => {
    const user = userEvent.setup();
    const { onAcknowledge } = renderPanel();

    await user.click(
      screen.getByRole("button", { name: "I saved it — finish" }),
    );

    expect(onAcknowledge).toHaveBeenCalledTimes(1);
  });

  it("disables the ack button and shows the pending label while acknowledging", () => {
    const { onAcknowledge } = renderPanel({ isAcknowledging: true });

    const button = screen.getByRole("button", { name: "Notifying CLI..." });
    expect(button).toBeDisabled();
    // No way to re-fire the pairing-complete POST while it's in flight.
    expect(onAcknowledge).not.toHaveBeenCalled();
  });
});

describe("RecoveryCodesPanel — one-time reveal + copy-all", () => {
  const CODES = ["AAAA-1111", "BBBB-2222", "CCCC-3333"];

  it("masks every code by default and reveals all on Reveal", async () => {
    const user = userEvent.setup();
    render(<RecoveryCodesPanel codes={CODES} onAcknowledged={vi.fn()} />);

    // None of the raw codes are visible until reveal.
    for (const code of CODES) {
      expect(screen.queryByText(code)).not.toBeInTheDocument();
    }

    await user.click(screen.getByRole("button", { name: "Reveal" }));
    for (const code of CODES) {
      expect(screen.getByText(code)).toBeInTheDocument();
    }
  });

  it("copies all codes newline-joined and flips the label to Copied!", async () => {
    const user = userEvent.setup();
    const writeText = spyClipboard();
    render(<RecoveryCodesPanel codes={CODES} onAcknowledged={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "Copy all" }));

    expect(writeText).toHaveBeenCalledWith(CODES.join("\n"));
    expect(await screen.findByText("Copied!")).toBeInTheDocument();
  });

  it("fires onAcknowledged when the user confirms they saved the codes", async () => {
    const user = userEvent.setup();
    const onAcknowledged = vi.fn();
    render(<RecoveryCodesPanel codes={CODES} onAcknowledged={onAcknowledged} />);

    await user.click(
      screen.getByRole("button", { name: "I have saved them — close" }),
    );

    expect(onAcknowledged).toHaveBeenCalledTimes(1);
  });
});
