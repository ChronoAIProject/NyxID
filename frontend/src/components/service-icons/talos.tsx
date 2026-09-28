import type { ServiceIconProps } from "./index";

export default function TalosIcon({ className }: ServiceIconProps) {
  return (
    <svg
      viewBox="0 0 424 424"
      className={className}
      fill="currentColor"
      data-slug="talos"
      aria-hidden="true"
    >
      <path d="M1 1H336C384 1 423 40 423 88V141H71C32 141 1 110 1 71Z" />
      <path d="M142 165H282V353C282 392 251 423 212 423H142Z" />
    </svg>
  );
}
