import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { TextBlock } from "./text-block";
import { isHostedConnectLink } from "@/lib/assistant/hosted-connect-link";

describe("hosted connect links", () => {
  it("renders an explicit same-origin connect action without auto-opening it", () => {
    const text = `[GitHub](${window.location.origin}/connect/nyx_clk_test) ` +
      "[Docs](https://example.com/help)";
    render(<TextBlock text={text} />);
    const connect = screen.getByRole("link", { name: "Connect GitHub" });
    expect(connect).toHaveAttribute("target", "_blank");
    expect(connect).toHaveAttribute("rel", "noopener noreferrer");
    expect(screen.getByRole("link", { name: "Docs" })).toHaveAttribute("rel", "noopener noreferrer");
  });
  it("admits only exact hosted link paths on this origin", () => {
    const origin = "https://nyxid.example";
    expect(isHostedConnectLink("/connect/nyx_clk_abc-123", origin)).toBe(true);
    for (const url of [
      "https://evil.example/connect/nyx_clk_abc",
      "/connect/abc",
      "/connect/nyx_clk_abc?redirect=evil",
      "/connect/nyx_clk_abc#x",
      "https://user@nyxid.example/connect/nyx_clk_abc",
      "javascript:alert(1)",
    ]) {
      expect(isHostedConnectLink(url, origin)).toBe(false);
    }
  });
});
