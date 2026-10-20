import type { ServiceIconProps } from "./index";

export default function AgentEventGatewayIcon({ className }: ServiceIconProps) {
  return (
    <svg
      viewBox="0 0 64 64"
      className={className}
      fill="currentColor"
      data-slug="cmaeg"
      aria-hidden="true"
    >
      <path d="M6 60V27L19 9L30 3.9V19L19 31V60Z" />
      <path d="M58 60V27L45 9L34 3.9V19L45 31V60Z" />
      <path d="M32 29L39 38V49L32 58L25 49V38Z" />
    </svg>
  );
}
