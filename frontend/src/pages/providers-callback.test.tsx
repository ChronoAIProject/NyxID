import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { mockNavigate, searchState } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
  // Mirrors the URL query params the backend redirects with
  // (?status=success | ?status=error&message=...).
  searchState: { value: {} as Record<string, unknown> },
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useSearch: () => searchState.value,
}));

import { ProvidersCallbackPage } from "./providers-callback";

describe("ProvidersCallbackPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    searchState.value = {};
  });

  it("shows the success state and confirmation copy when status=success", () => {
    searchState.value = { status: "success" };

    render(<ProvidersCallbackPage />);

    expect(
      screen.getByText("Provider Connected"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Your provider has been connected successfully."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Connection Failed")).not.toBeInTheDocument();
  });

  it("navigates back to providers from the success state button", async () => {
    const user = userEvent.setup();
    searchState.value = { status: "success" };

    render(<ProvidersCallbackPage />);

    await user.click(screen.getByRole("button", { name: "Back to Providers" }));

    expect(mockNavigate).toHaveBeenCalledWith({ to: "/providers" });
  });

  it("shows the failure state with the provided error message when status=error", () => {
    searchState.value = { status: "error", message: "provider rejected scope" };

    render(<ProvidersCallbackPage />);

    expect(screen.getByText("Connection Failed")).toBeInTheDocument();
    expect(screen.getByText("provider rejected scope")).toBeInTheDocument();
    expect(screen.queryByText("Provider Connected")).not.toBeInTheDocument();
  });

  it("falls back to the default error message when status=error has no message", () => {
    searchState.value = { status: "error" };

    render(<ProvidersCallbackPage />);

    expect(screen.getByText("Connection Failed")).toBeInTheDocument();
    expect(screen.getByText("OAuth connection failed")).toBeInTheDocument();
  });

  it("treats a missing status param as the failure (non-success) state", () => {
    searchState.value = {};

    render(<ProvidersCallbackPage />);

    // status is neither "success" nor "error": title is the failure title and
    // errorMessage stays null, so no message paragraph is shown.
    expect(screen.getByText("Connection Failed")).toBeInTheDocument();
    expect(screen.queryByText("Provider Connected")).not.toBeInTheDocument();
    expect(screen.queryByText("OAuth connection failed")).not.toBeInTheDocument();
  });
});
