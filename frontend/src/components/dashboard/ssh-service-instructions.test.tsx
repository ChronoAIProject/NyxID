import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { deriveNyxidBaseUrl } from "@/lib/ssh";
import { SshServiceInstructions } from "./ssh-service-instructions";

const mocks = vi.hoisted(() => ({
  usePublicConfig: vi.fn(),
}));

vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: mocks.usePublicConfig,
}));

describe("deriveNyxidBaseUrl", () => {
  beforeEach(() => {
    window.history.replaceState({}, "", "/app");
  });

  it("converts the node websocket URL into an https base URL", () => {
    expect(deriveNyxidBaseUrl("wss://auth.nyxid.test/api/v1/nodes/ws")).toBe(
      "https://auth.nyxid.test",
    );
  });

  it("falls back to the browser origin when the websocket URL is invalid", () => {
    expect(deriveNyxidBaseUrl("not-a-url")).toBe(window.location.origin);
  });
});

describe("SshServiceInstructions", () => {
  beforeEach(() => {
    mocks.usePublicConfig.mockReturnValue({
      data: {
        node_ws_url: "wss://auth.nyxid.test/api/v1/nodes/ws",
      },
    });
  });

  it("renders copyable SSH helper commands with the derived NyxID base URL", () => {
    render(
      <SshServiceInstructions
        serviceId="svc-1"
        serviceSlug="prod-shell"
        sshConfig={{
          host: "ssh.internal.example",
          port: 22,
          certificate_auth_enabled: true,
          certificate_ttl_minutes: 30,
          allowed_principals: ["ubuntu"],
          ca_public_key: "ssh-ed25519 AAAAtest",
        }}
      />,
    );

    expect(screen.getByText("Client Setup")).toBeInTheDocument();
    expect(screen.getByText("1. Install CLI")).toBeInTheDocument();
    expect(
      screen.getAllByText((content) =>
        content.includes(
          "--base-url https://auth.nyxid.test --service-id svc-1",
        ),
      ).length,
    ).toBeGreaterThanOrEqual(2);
    expect(screen.getByText("Optional: Generate SSH config stanza")).toBeInTheDocument();
    expect(screen.getByText("Target Machine Setup (Passwordless Login)")).toBeInTheDocument();
    expect(screen.getByText("Node Agent (Remote Targets)")).toBeInTheDocument();
  });
});
