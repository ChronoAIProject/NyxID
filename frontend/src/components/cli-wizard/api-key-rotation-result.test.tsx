import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { ApiKeyRotationResult } from "./api-key-rotation-result";

it("explains a private assistant rotation and closes without secret-copy controls", async () => {
  const user = userEvent.setup();
  const close = vi.fn();
  render(<ApiKeyRotationResult
    result={{
      kind: "api-key-rotate",
      resource_id: "successor",
      full_key: "",
      platform: "nyxid-assistant",
    }}
    description="Save this new value now."
    ackButtonLabel="I have saved this — close"
    onAcknowledge={close}
  />);

  expect(screen.getByText(/Key rotated\. This key is managed/)).toHaveTextContent(
    "Key rotated. This key is managed by the NyxID assistant; " +
    "the new secret is stored encrypted on the server and is never shown.",
  );
  expect(screen.queryByText("Shown once — save it now")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /reveal|copy|saved/i })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Close" }));
  expect(close).toHaveBeenCalledOnce();
});

it.each([undefined, "nyxid-assistant"])(
  "retains one-time display for a nonempty key with platform %s",
  async (platform) => {
    const user = userEvent.setup();
    render(<ApiKeyRotationResult
      result={{
        kind: "api-key-rotate",
        resource_id: "successor",
        full_key: "ordinary-rotation-secret",
        platform,
      }}
      description="Save this new value now."
      ackButtonLabel="I have saved this — close"
      onAcknowledge={vi.fn()}
    />);

    expect(screen.getByText("Shown once — save it now")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /reveal/i }));
    expect(screen.getByText("ordinary-rotation-secret")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "I have saved this — close" })).toBeInTheDocument();
  },
);
