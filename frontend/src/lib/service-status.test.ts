import { describe, expect, it } from "vitest";

import { deriveServiceBadge } from "./service-status";

describe("deriveServiceBadge", () => {
  it("marks a fully-ready service as Active", () => {
    const result = deriveServiceBadge({
      isActive: true,
      credentialStatus: "active",
      hasCredential: true,
    });
    expect(result).toEqual({
      variant: "success",
      label: "Active",
      credentialBlocked: false,
    });
  });

  it("marks a deactivated service as Inactive regardless of credential", () => {
    expect(
      deriveServiceBadge({
        isActive: false,
        credentialStatus: "active",
        hasCredential: true,
      }),
    ).toEqual({
      variant: "secondary",
      label: "Inactive",
      credentialBlocked: false,
    });

    // Even when the credential is broken, an explicitly-deactivated service
    // should read as Inactive rather than Unavailable: the user already knows
    // they turned it off.
    expect(
      deriveServiceBadge({
        isActive: false,
        credentialStatus: "pending_auth",
        hasCredential: true,
      }),
    ).toMatchObject({ label: "Inactive" });
  });

  it("marks a pending_auth credential as Unavailable (NyxID#329)", () => {
    // Repro for the bug: after switching Route via Node -> Direct without a
    // direct credential, UserApiKey.status becomes "pending_auth" but
    // UserService.is_active stays true. The badge must reflect the real
    // failure state instead of showing "Active".
    const result = deriveServiceBadge({
      isActive: true,
      credentialStatus: "pending_auth",
      hasCredential: true,
    });
    expect(result).toEqual({
      variant: "secondary",
      label: "Unavailable",
      credentialBlocked: true,
    });
  });

  it.each(["expired", "revoked", "refresh_failed"])(
    "marks a %s credential as Unavailable",
    (status) => {
      const result = deriveServiceBadge({
        isActive: true,
        credentialStatus: status,
        hasCredential: true,
      });
      expect(result).toMatchObject({
        label: "Unavailable",
        credentialBlocked: true,
      });
    },
  );

  it("ignores credential status for services without a credential", () => {
    // Auto-connected / no-auth services don't have a UserApiKey attached.
    // Treating an empty status as "not ready" would falsely flag them.
    const result = deriveServiceBadge({
      isActive: true,
      credentialStatus: "",
      hasCredential: false,
    });
    expect(result).toEqual({
      variant: "success",
      label: "Active",
      credentialBlocked: false,
    });
  });
});
