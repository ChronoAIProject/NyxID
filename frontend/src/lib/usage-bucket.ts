export interface UsageBucketLabelInput {
  readonly date: string;
  readonly request_count: number;
  readonly error_count: number;
}

export function formatBucketDate(iso: string): string {
  const parts = iso.split("-");
  if (parts.length !== 3) {
    return iso;
  }
  const year = Number(parts[0]);
  const month = Number(parts[1]);
  const day = Number(parts[2]);
  if (Number.isNaN(year) || Number.isNaN(month) || Number.isNaN(day)) {
    return iso;
  }
  const date = new Date(year, month - 1, day);
  return date.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

export function formatBucketLabel(bucket: UsageBucketLabelInput): string {
  const requestLabel = bucket.request_count === 1 ? "request" : "requests";
  const base = `${formatBucketDate(bucket.date)}: ${bucket.request_count} ${requestLabel}`;
  if (bucket.error_count > 0) {
    const errorLabel = bucket.error_count === 1 ? "error" : "errors";
    return `${base}, ${bucket.error_count} ${errorLabel}`;
  }
  return base;
}
