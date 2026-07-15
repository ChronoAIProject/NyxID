import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { mockNavigate } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

import { OAuthErrorPage } from "./oauth-error";

function setSearch(query: string) {
  window.history.pushState({}, "", `/oauth/error${query}`);
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  window.history.pushState({}, "", "/");
});

describe("OAuthErrorPage", () => {
  it("maps a known error code to its friendly title and shows the raw code", () => {
    setSearch("?code=invalid_scope&message=The%20requested%20scope%20is%20invalid");

    render(<OAuthErrorPage />);

    // ERROR_LABELS["invalid_scope"] === "Invalid Scope" (rendered as CardTitle).
    expect(screen.getByText("Invalid Scope")).toBeInTheDocument();
    // The provided message renders verbatim.
    expect(
      screen.getByText("The requested scope is invalid"),
    ).toBeInTheDocument();
    // The raw error code is surfaced under the "Error code" label.
    const codeBlock = screen.getByText("Error code").parentElement!;
    expect(within(codeBlock).getByText("invalid_scope")).toBeInTheDocument();
  });

  it("falls back to the generic title for an unrecognized error code", () => {
    setSearch("?code=teapot&message=Short%20and%20stout");

    render(<OAuthErrorPage />);

    expect(screen.getByText("Authorization Error")).toBeInTheDocument();
    // The unknown code is still shown verbatim.
    expect(screen.getByText("teapot")).toBeInTheDocument();
  });

  it("uses default code and message when params are absent", () => {
    setSearch("");

    render(<OAuthErrorPage />);

    // No code -> "unknown_error" -> generic "Authorization Error" title.
    expect(screen.getByText("Authorization Error")).toBeInTheDocument();
    expect(screen.getByText("unknown_error")).toBeInTheDocument();
    expect(
      screen.getByText("An unexpected error occurred during authorization."),
    ).toBeInTheDocument();
  });

  it("links a missing required service to the existing connection flow", async () => {
    const user = userEvent.setup();
    const backSpy = vi
      .spyOn(window.history, "back")
      .mockImplementation(() => undefined);
    setSearch(
      "?code=required_service_not_connected" +
        "&message=Required%20service%20is%20not%20connected" +
        "&service_slug=chrono-llm-public" +
        "&service_name=Chrono%20Public%20LLM",
    );

    render(<OAuthErrorPage />);

    expect(screen.getByText("Connect a Required Service")).toBeInTheDocument();
    expect(screen.getByText("Chrono Public LLM")).toBeInTheDocument();
    expect(screen.getByText("chrono-llm-public")).toBeInTheDocument();
    expect(
      screen.getByText(/is required by this app but is not connected/),
    ).toBeInTheDocument();

    const connect = screen.getByRole("link", { name: /Connect service/ });
    expect(connect).toHaveAttribute(
      "href",
      "/keys?tab=services&slug=chrono-llm-public",
    );
    expect(connect).toHaveAttribute("target", "_blank");

    await user.click(
      screen.getByRole("button", { name: /Retry authorization/ }),
    );
    expect(backSpy).toHaveBeenCalledTimes(1);
    backSpy.mockRestore();
  });

  it("Go Back calls window.history.back()", async () => {
    const user = userEvent.setup();
    setSearch("?code=login_required");
    const backSpy = vi.spyOn(window.history, "back").mockImplementation(() => undefined);

    render(<OAuthErrorPage />);

    await user.click(screen.getByRole("button", { name: "Go Back" }));
    expect(backSpy).toHaveBeenCalledTimes(1);
    backSpy.mockRestore();
  });

  it("Home navigates to /dashboard", async () => {
    const user = userEvent.setup();
    setSearch("?code=login_required");

    render(<OAuthErrorPage />);

    await user.click(screen.getByRole("button", { name: "Home" }));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashboard" });
  });

  it("maps each documented error code to its label", () => {
    const cases: Record<string, string> = {
      invalid_request: "Invalid Request",
      invalid_redirect_uri: "Invalid Redirect URI",
      not_found: "Client Not Found",
      bad_request: "Bad Request",
      pkce_verification_failed: "PKCE Verification Failed",
      consent_required: "Consent Required",
      login_required: "Login Required",
      required_service_not_connected: "Connect a Required Service",
    };
    for (const [code, label] of Object.entries(cases)) {
      setSearch(`?code=${code}`);
      const { unmount } = render(<OAuthErrorPage />);
      // CardTitle is a styled div, so match the label by text, not role.
      expect(screen.getByText(label)).toBeInTheDocument();
      // The code block also reflects the same code.
      const codeBlock = screen.getByText("Error code").parentElement!;
      expect(within(codeBlock).getByText(code)).toBeInTheDocument();
      unmount();
    }
  });
});
