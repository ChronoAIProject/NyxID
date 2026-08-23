import { describe, expect, it } from "vitest";
import { ACTION_DIALOGS } from "@/components/assistant/blocks/action-dialogs";
import manifestActions from "@/lib/assistant/__fixtures__/assistant-actions-manifest.json";
import { ACTION_REGISTRY } from "@/lib/assistant/action-registry";

const ACTIVE_WIRING = {
  "service.connect": "legacy_connect",
  "service.reauthorize": "legacy_reauthorize",
  "key.create": "dialog",
  "key.rotate": "dialog",
  "key.update": "dialog",
  "key.delete": "dialog",
  "key.extend_scope": "dialog",
  "key.bind_credential": "dialog",
} as const;

describe("assistant action registry", () => {
  it("registry_covers_every_manifest_verb", () => {
    // Falsifier exercised: deleting the `openclaw.connect` registry row makes
    // this closure check fail instead of silently leaving the verb uncovered.
    expect(manifestActions).toHaveLength(54);
    expect(new Set(manifestActions).size).toBe(54);
    expect(Object.keys(ACTION_REGISTRY).sort()).toEqual(
      [...manifestActions].sort(),
    );

    for (const action of manifestActions) {
      const descriptor = ACTION_REGISTRY[action];
      expect(descriptor, `${action} must have a registry row`).toBeDefined();
      if (!descriptor) continue;

      const expectedWiring =
        ACTIVE_WIRING[action as keyof typeof ACTIVE_WIRING] ?? "deferred";
      expect(
        descriptor.wiring,
        `${action} must not change wiring without an explicit review update`,
      ).toBe(expectedWiring);

      const variant = action.replaceAll(".", "_");
      const hasDialog = Object.prototype.hasOwnProperty.call(
        ACTION_DIALOGS,
        variant,
      );
      if (descriptor.wiring === "dialog") {
        expect(hasDialog, `${action} must have an ACTION_DIALOGS binding`).toBe(
          true,
        );
      } else {
        expect(
          hasDialog,
          `${action} must stay unbound while its wiring is ${descriptor.wiring}`,
        ).toBe(false);
      }
    }
  });
});
