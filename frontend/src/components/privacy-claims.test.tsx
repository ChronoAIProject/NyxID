import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { appI18n } from "@/i18n";
import en from "@/i18n/locales/app/en-full";
import zhCN from "@/i18n/locales/app/zh-CN";
import zhTW from "@/i18n/locales/app/zh-TW";
import { AuthLayout } from "./layout/auth-layout";
import { OnboardingTakeover } from "./dashboard/onboarding-takeover";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children }: { to: string; children: ReactNode }) => <a href={to}>{children}</a>,
  Outlet: () => null,
  useNavigate: () => vi.fn(),
}));
vi.mock("@/hooks/use-logo-href", () => ({ useLogoHref: () => "/" }));
vi.mock("@/hooks/use-theme", () => ({ useApplyTheme: () => {}, useResolvedTheme: () => "dark" }));
vi.mock("@/hooks/use-onboarding", () => ({
  useCompleteOnboarding: () => ({ isPending: false, mutateAsync: vi.fn() }),
}));
vi.mock("@/components/shared/github-button", () => ({ GitHubButton: () => null }));
vi.mock("@/components/dashboard/add-key-dialog", () => ({ AddKeyDialog: () => null }));

afterEach(async () => {
  cleanup();
  await appI18n.changeLanguage("en");
});

it.each([
  ["en", en, "Privacy Policy", ".", "Encrypted at rest"],
  ["zh-CN", zhCN, "隐私政策", "。", "静态加密"],
  ["zh-TW", zhTW, "隱私權政策", "。", "靜態加密"],
] as const)("renders accurate account privacy claims in %s", async (language, catalog, privacy, period, encryption) => {
  await appI18n.changeLanguage(language);
  render(<AuthLayout />);
  const footer = screen.getByRole("link", { name: privacy }).closest("p")!;
  expect(footer.textContent?.trim().endsWith(privacy + period)).toBe(true);
  expect(catalog).not.toHaveProperty(", and to receive periodic emails with updates.");
  expect(footer.textContent).not.toMatch(/periodic emails|更新电子邮件|更新電子郵件/);
  cleanup();

  render(<OnboardingTakeover />);
  expect(screen.getByText(encryption)).toBeVisible();
  expect(screen.getByText(encryption).querySelector("svg")).toBeInTheDocument();
  expect(screen.getByText(catalog["Zero credential exposure"]!)).toBeVisible();
  expect(catalog).not.toHaveProperty("End-to-end encrypted");
  expect(document.body).not.toHaveTextContent(/End-to-end encrypted|端到端加密|端對端加密/);
});
