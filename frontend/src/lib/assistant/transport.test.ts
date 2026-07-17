import { describe, expect, it } from "vitest";
import {
  assistantTransport,
  selectAssistantTransportKind,
} from "@/lib/assistant/transport";
import { AevatarAssistantTransport } from "@/lib/assistant/aevatar-transport";

describe("selectAssistantTransportKind", () => {
  it("keeps vitest sessions on the scripted mock", () => {
    expect(
      selectAssistantTransportKind({ mode: "test", dev: false, search: "" }),
    ).toBe("mock");
  });

  it("uses the mock for dev sessions that opt in with ?mock", () => {
    expect(
      selectAssistantTransportKind({
        mode: "development",
        dev: true,
        search: "?mock",
      }),
    ).toBe("mock");
  });

  it("talks to aevatar for production and plain dev sessions", () => {
    expect(
      selectAssistantTransportKind({
        mode: "production",
        dev: false,
        search: "",
      }),
    ).toBe("aevatar");
    expect(
      selectAssistantTransportKind({ mode: "development", dev: true, search: "" }),
    ).toBe("aevatar");
  });
});

describe("session transport", () => {
  it("never instantiates the aevatar transport for a vitest session", () => {
    // The test session resolves to the scripted mock, not the live transport.
    expect(assistantTransport).not.toBeInstanceOf(AevatarAssistantTransport);
  });
});
