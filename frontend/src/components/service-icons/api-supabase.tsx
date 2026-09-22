import type { ServiceIconProps } from "./index";

export default function ApiSupabaseIcon({ className }: ServiceIconProps) {
  return (
    <svg
      viewBox="0 0 512 512"
      fill="currentColor"
      aria-hidden="true"
      data-slug="api-supabase"
      className={className}
    >
      <path d="M297.6 501c-12.9 16.3-39.2 7.4-39.5-13.4L253.6 183h204.8c37.1 0 57.8 42.8 34.7 71.9z" />
      <path d="M214.4 11c12.9-16.3 39.2-7.4 39.5 13.4l2 304.5H53.7c-37.1 0-57.8-42.8-34.7-71.9z" />
    </svg>
  );
}
