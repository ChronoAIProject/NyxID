import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

// Issue #787: this module is a pure re-export that preserves the legacy
// `@/pages/cli-pair/display-once` import path while the implementation
// lives in the shared cli-wizard directory. The contract to pin is the
// re-export wiring itself: callers importing from this path must get the
// *same* components the wizard bundle uses — not stale forks.

import * as reexport from "./display-once";
import {
  DisplayOncePanel as SourcePanel,
  RecoveryCodesPanel as SourceCodes,
} from "@/components/cli-wizard/display-once-panel";

describe("cli-pair/display-once re-export", () => {
  it("re-exports the identical component references from cli-wizard", () => {
    // Identity check: a divergent re-export (e.g. a wrapper) would break
    // this, which is exactly the regression this thin module guards.
    expect(reexport.DisplayOncePanel).toBe(SourcePanel);
    expect(reexport.RecoveryCodesPanel).toBe(SourceCodes);
  });

  it("renders a working DisplayOncePanel through the legacy path", () => {
    render(
      <reexport.DisplayOncePanel
        title="Legacy path token"
        description="d"
        secret="nyx_secret_123"
        ackButtonLabel="ack"
        onAcknowledge={vi.fn()}
        isAcknowledging={false}
      />,
    );

    expect(screen.getByText("Legacy path token")).toBeInTheDocument();
    expect(screen.getByText("Shown once — save it now")).toBeInTheDocument();
  });
});
