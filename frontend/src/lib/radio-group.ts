import type { KeyboardEvent } from "react";

export function radioKeys(event: KeyboardEvent<HTMLButtonElement>) {
  if (!["ArrowDown", "ArrowRight", "ArrowUp", "ArrowLeft", "Home", "End"].includes(event.key))
    return;
  event.preventDefault();
  const rows = [
    ...(event.currentTarget
      .closest('[role="radiogroup"]')
      ?.querySelectorAll<HTMLButtonElement>('[role="radio"]:not(:disabled)') ?? []),
  ];
  const index = rows.indexOf(event.currentTarget);
  const next =
    event.key === "Home"
      ? 0
      : event.key === "End"
        ? rows.length - 1
        : (index +
            (event.key === "ArrowDown" || event.key === "ArrowRight" ? 1 : -1) +
            rows.length) %
          rows.length;
  rows[next]?.focus();
  rows[next]?.click();
}
