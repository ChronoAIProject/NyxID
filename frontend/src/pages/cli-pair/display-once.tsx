// Re-export: the implementation lives in the shared cli-wizard directory so
// the locally-served wizard bundle (Mode A) and the remote-pairing page
// (Mode B, `/cli/pair`) render the same DisplayOnce panel. This module
// preserves the existing `@/pages/cli-pair/display-once` import path for
// callers that were already using it.

export { DisplayOncePanel } from "@/components/cli-wizard/display-once-panel"
