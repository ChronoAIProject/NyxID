import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  useSystemSkillSearch,
  useSystemSkillVersions,
  useSystemSkillPreview,
} from "./use-assistant-system-skills";

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: vi.fn(),
}));
afterEach(() => vi.restoreAllMocks());

it("uses admin catalog routes for all picker reads when the personal engine flag is off", async () => {
  vi.mocked(useAuthStore).mockImplementation((selector) =>
    selector({ user: { id: "admin", capabilities: { enabled_features: [] } } } as never),
  );
  const base = "/admin/assistant/system-skills";
  const reference = {
    source: "ornn",
    skill_id: "skill-id",
    name: "Guide",
    version: "1.0",
    sha256: "a".repeat(64),
    dependencies: [],
    disabled: false,
  };
  const get = vi.spyOn(api, "get").mockImplementation(async (path) => {
    if (path.endsWith("/preview"))
      return { reference, description: "Guide", size_bytes: 100 };
    if (path.includes("/versions?"))
      return {
        items: [
          { version: "1.0", sha256: reference.sha256, deprecated: false },
        ],
        page: 1,
        total_pages: 1,
      };
    return {
      items: [{ id: "skill-id", name: "Guide", description: "Guide" }],
      page: 2,
      total_pages: 2,
    };
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  try {
    const { result } = renderHook(
      () => ({
        search: useSystemSkillSearch("public guide", 2),
        versions: useSystemSkillVersions("skill-id", 1),
        preview: useSystemSkillPreview("skill-id", "1.0"),
      }),
      { wrapper },
    );
    await waitFor(() => {
      expect(result.current.search.isSuccess).toBe(true);
      expect(result.current.versions.isSuccess).toBe(true);
      expect(result.current.preview.isSuccess).toBe(true);
    });
    expect(get.mock.calls.map(([path]) => path).sort()).toEqual(
      [
        `${base}/search?q=public%20guide&page=2`,
        `${base}/skill-id/versions?page=1`,
        `${base}/skill-id/versions/1.0/preview`,
      ].sort(),
    );
  } finally {
    client.clear();
  }
});
