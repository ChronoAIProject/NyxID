import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { CrmPage } from "./crm-page";

vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => false,
}));

const initialAuth = useAuthStore.getState();

beforeEach(() => {
  useAuthStore.setState({ isLoading: false });
  vi.stubGlobal("fetch", vi.fn());
});

afterEach(() => {
  cleanup();
  useAuthStore.setState(initialAuth);
  vi.unstubAllGlobals();
});

it("shows the unavailable state without issuing CRM requests", () => {
  render(<CrmPage />);

  expect(
    screen.getByText("CRM isn't available for your account."),
  ).toBeInTheDocument();
  expect(fetch).not.toHaveBeenCalled();
});
