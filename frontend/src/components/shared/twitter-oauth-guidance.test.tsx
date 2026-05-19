import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TwitterOAuthGuidance } from "./twitter-oauth-guidance";

const mocks = vi.hoisted(() => ({
  copyToClipboard: vi.fn(),
  toastSuccess: vi.fn(),
  useRuntimeConfig: vi.fn(),
}));

vi.mock("@/hooks/use-runtime-config", () => ({
  useRuntimeConfig: mocks.useRuntimeConfig,
}));

vi.mock("@/lib/utils", async () => {
  const actual = await vi.importActual<typeof import("@/lib/utils")>(
    "@/lib/utils",
  );
  return {
    ...actual,
    copyToClipboard: mocks.copyToClipboard,
  };
});

vi.mock("sonner", () => ({
  toast: {
    success: mocks.toastSuccess,
  },
}));

describe("TwitterOAuthGuidance", () => {
  beforeEach(() => {
    mocks.copyToClipboard.mockReset();
    mocks.copyToClipboard.mockResolvedValue(undefined);
    mocks.toastSuccess.mockReset();
    mocks.useRuntimeConfig.mockReset();
  });

  it("renders a loading state while fetching the runtime callback URL", () => {
    mocks.useRuntimeConfig.mockReturnValue({
      data: undefined,
      isError: false,
      isLoading: true,
    });

    render(<TwitterOAuthGuidance slug="twitter" />);

    expect(screen.getByText("Loading callback URL...")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Copy callback URL" }),
    ).not.toBeInTheDocument();
  });

  it("renders a copyable provider callback URL from runtime config", async () => {
    const user = userEvent.setup();
    mocks.useRuntimeConfig.mockReturnValue({
      data: {
        api_base_url: "https://nyx-api.chrono-ai.fun",
      },
      isError: false,
      isLoading: false,
    });

    render(<TwitterOAuthGuidance slug="twitter" />);

    const callbackUrl =
      "https://nyx-api.chrono-ai.fun/api/v1/providers/callback";
    expect(screen.getByText(callbackUrl)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Copy callback URL" }));

    expect(mocks.copyToClipboard).toHaveBeenCalledWith(callbackUrl);
    await waitFor(() => {
      expect(mocks.toastSuccess).toHaveBeenCalledWith("Callback URL copied");
    });
  });

  it("renders a user-facing fallback when runtime config fails to load", () => {
    mocks.useRuntimeConfig.mockReturnValue({
      data: undefined,
      isError: true,
      isLoading: false,
    });

    render(<TwitterOAuthGuidance slug="api-twitter" />);

    expect(
      screen.getByText(
        /Couldn't load callback URL\. Please retry\. If this persists, contact support\./,
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/VITE_BACKEND_URL/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Copy callback URL" }),
    ).not.toBeInTheDocument();
  });

  it("does not render for non-Twitter providers", () => {
    mocks.useRuntimeConfig.mockReturnValue({
      data: {
        api_base_url: "https://nyx-api.chrono-ai.fun",
      },
      isError: false,
      isLoading: false,
    });

    const { container } = render(<TwitterOAuthGuidance slug="github" />);

    expect(container).toBeEmptyDOMElement();
    expect(mocks.useRuntimeConfig).not.toHaveBeenCalled();
  });
});
