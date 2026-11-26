import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { NodeStatusBadge } from "./node-status-badge";

afterEach(cleanup);

it.each([
  ["online", true, "Online"],
  ["online", false, "Offline"],
  ["offline", false, "Offline"],
  ["draining", false, "Draining"],
  ["draining", true, "Online"],
] as const)("renders %s with connection %s as %s", (status, connected, expected) => {
  render(<NodeStatusBadge status={status} isConnected={connected} />);
  expect(screen.getByText(expected)).toBeInTheDocument();
});
