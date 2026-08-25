import { describe, expect, it } from "vitest";
import { ACTION_DIALOGS } from "@/components/assistant/blocks/action-dialogs";
import manifestActions from "@/lib/assistant/__fixtures__/assistant-actions-manifest.json";
import { ACTION_REGISTRY } from "@/lib/assistant/action-registry";

const NON_DIALOG_WIRING = {
  "service.connect": "legacy_connect",
  "service.reauthorize": "legacy_reauthorize",
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
        NON_DIALOG_WIRING[action as keyof typeof NON_DIALOG_WIRING] ?? "dialog";
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
