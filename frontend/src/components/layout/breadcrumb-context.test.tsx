import { renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { BreadcrumbLabelContext, useBreadcrumbLabel, useBreadcrumbSection } from "./breadcrumb-context";

describe("breadcrumb labels", () => {
  it("registers labels under the current route and clears them on unmount", () => {
    const setLabel = vi.fn();
    const wrapper = ({ children }: { children: ReactNode }) => (
      <BreadcrumbLabelContext.Provider value={{ pathname: "/keys/key-1", labels: {}, setLabel }}>
        {children}
      </BreadcrumbLabelContext.Provider>
    );
    const { unmount } = renderHook(() => useBreadcrumbLabel("Production key"), { wrapper });
    expect(setLabel).toHaveBeenLastCalledWith("/keys/key-1", "Production key");
    unmount();
    expect(setLabel).toHaveBeenLastCalledWith("/keys/key-1", null);
  });

  it("keeps parent and child labels separate", () => {
    const setLabel = vi.fn();
    const wrapper = ({ children }: { children: ReactNode }) => (
      <BreadcrumbLabelContext.Provider value={{ pathname: "/orgs/org-1/service-accounts/sa-1", labels: {}, setLabel }}>
        {children}
      </BreadcrumbLabelContext.Provider>
    );
    const { unmount } = renderHook(() => {
      useBreadcrumbLabel("Engineering", "/orgs/org-1");
      useBreadcrumbLabel("Worker");
    }, { wrapper });
    expect(setLabel).toHaveBeenCalledWith("/orgs/org-1", "Engineering");
    expect(setLabel).toHaveBeenCalledWith("/orgs/org-1/service-accounts/sa-1", "Worker");
    unmount();
    expect(setLabel).toHaveBeenCalledWith("/orgs/org-1", null);
    expect(setLabel).toHaveBeenCalledWith("/orgs/org-1/service-accounts/sa-1", null);
  });

  it("updates the visible section and clears it on unmount", () => {
    const setSection = vi.fn();
    const wrapper = ({ children }: { children: ReactNode }) => (
      <BreadcrumbLabelContext.Provider value={{ pathname: "/keys/key-1", labels: {}, setLabel: vi.fn(), setSection }}>
        {children}
      </BreadcrumbLabelContext.Provider>
    );
    const { rerender, unmount } = renderHook(({ tab }) => useBreadcrumbSection(tab), {
      wrapper, initialProps: { tab: "overview" },
    });
    expect(setSection).toHaveBeenLastCalledWith("/keys/key-1", "overview");
    rerender({ tab: "advanced" });
    expect(setSection).toHaveBeenLastCalledWith("/keys/key-1", "advanced");
    unmount();
    expect(setSection).toHaveBeenLastCalledWith("/keys/key-1", null);
  });
});
