import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { NodeStatusBadge } from "./node-status-badge";
import { nodeStatusKey } from "@/lib/status-contract";

vi.mock("./status-badge", () => ({ StatusBadge: ({ statusKey }: { statusKey: string }) => <span>{statusKey}</span> }));
afterEach(cleanup);

it.each([
  ["online", true, "Online"],
  ["online", false, "Offline"],
  ["offline", false, "Offline"],
  ["draining", false, "Draining"],
  ["draining", true, "Online"],
] as const)("derives %s with connection %s as %s for both consumers", (status, connected, expected) => {
  expect(nodeStatusKey(status, connected)).toBe(expected);
  render(<NodeStatusBadge status={status} isConnected={connected} />);
  expect(screen.getByText(expected)).toBeInTheDocument();
});
