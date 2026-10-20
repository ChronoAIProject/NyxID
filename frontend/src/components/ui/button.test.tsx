import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Button } from "./button";

describe("Button", () => {
  it.each(["default", "sm", "lg", "icon"] as const)(
    "preserves white primary text at size %s",
    (size) => {
      render(
        <Button variant="primary" size={size}>
          Connect
        </Button>,
      );

      expect(screen.getByRole("button", { name: "Connect" })).toHaveClass(
        "text-white",
        "text-12",
      );
    },
  );
});
