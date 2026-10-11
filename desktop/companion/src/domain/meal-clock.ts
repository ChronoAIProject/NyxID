import type { MealId, MealSchedule } from "./companion";

const MINUTE_MS = 60_000;
const DAY_MS = 24 * 60 * MINUTE_MS;

interface ZonedParts {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
  second: number;
}

export interface MealOccurrence {
  mealId: MealId;
  label: string;
  dateKey: string;
  scheduledAt: string;
}

export type DueLabel =
  | { kind: "now" }
  | { kind: "overdue"; minutes: number }
  | { kind: "in_minutes"; minutes: number }
  | { kind: "in_hours"; hours: number }
  | { kind: "tomorrow"; time: string }
  | { kind: "on_date"; dateKey: string; time: string };

const formatterCache = new Map<string, Intl.DateTimeFormat>();

function formatter(timezone: string): Intl.DateTimeFormat {
  const cached = formatterCache.get(timezone);
  if (cached) {
    return cached;
  }
  const created = new Intl.DateTimeFormat("en-CA", {
    timeZone: timezone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  });
  formatterCache.set(timezone, created);
  return created;
}

function zonedParts(instant: Date, timezone: string): ZonedParts {
  const values: Partial<Record<Intl.DateTimeFormatPartTypes, number>> = {};
  for (const part of formatter(timezone).formatToParts(instant)) {
    if (part.type !== "literal") {
      values[part.type] = Number(part.value);
    }
  }
  return {
    year: values.year ?? 0,
    month: values.month ?? 0,
    day: values.day ?? 0,
    hour: values.hour === 24 ? 0 : (values.hour ?? 0),
    minute: values.minute ?? 0,
    second: values.second ?? 0,
  };
}

function padded(value: number): string {
  return value.toString().padStart(2, "0");
}

function dateKeyFromParts(parts: ZonedParts): string {
  return `${parts.year.toString().padStart(4, "0")}-${padded(parts.month)}-${padded(parts.day)}`;
}

function comparableMinute(parts: ZonedParts): number {
  return Date.UTC(
    parts.year,
    parts.month - 1,
    parts.day,
    parts.hour,
    parts.minute,
  );
}

function parseDateKey(dateKey: string): [number, number, number] {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(dateKey);
  if (!match) {
    throw new Error(`Invalid date key: ${dateKey}`);
  }
  const parts: [number, number, number] = [
    Number(match[1]),
    Number(match[2]),
    Number(match[3]),
  ];
  const parsed = new Date(Date.UTC(parts[0], parts[1] - 1, parts[2]));
  if (
    parsed.getUTCFullYear() !== parts[0] ||
    parsed.getUTCMonth() !== parts[1] - 1 ||
    parsed.getUTCDate() !== parts[2]
  ) {
    throw new Error(`Invalid date key: ${dateKey}`);
  }
  return parts;
}

function parseTime(time: string): [number, number] {
  const match = /^(?:[01]\d|2[0-3]):[0-5]\d$/.exec(time);
  if (!match) {
    throw new Error(`Invalid meal time: ${time}`);
  }
  const [hour, minute] = time.split(":").map(Number);
  return [hour ?? 0, minute ?? 0];
}

export function dateKeyAt(instant: Date, timezone: string): string {
  return dateKeyFromParts(zonedParts(instant, timezone));
}

export function addCalendarDays(dateKey: string, days: number): string {
  const [year, month, day] = parseDateKey(dateKey);
  const shifted = new Date(Date.UTC(year, month - 1, day + days));
  return `${shifted.getUTCFullYear().toString().padStart(4, "0")}-${padded(
    shifted.getUTCMonth() + 1,
  )}-${padded(shifted.getUTCDate())}`;
}

/**
 * Converts a wall-clock meal time into an instant without relying on the host
 * timezone. Ambiguous fall-back minutes select the first occurrence. A time in
 * a spring-forward gap advances to the first real minute after that gap.
 */
export function instantForLocalTime(
  dateKey: string,
  time: string,
  timezone: string,
): Date {
  const [year, month, day] = parseDateKey(dateKey);
  const [hour, minute] = parseTime(time);
  const desired = Date.UTC(year, month - 1, day, hour, minute);

  const offsets = new Set(
    [desired - DAY_MS, desired, desired + DAY_MS].map((timestamp) => {
      const parts = zonedParts(new Date(timestamp), timezone);
      return comparableMinute(parts) - timestamp;
    }),
  );

  // Only a minute skipped by a forward DST transition enters this scan.
  for (let shiftedMinutes = 0; shiftedMinutes <= 180; shiftedMinutes += 1) {
    const targetLocal = desired + shiftedMinutes * MINUTE_MS;
    const matches = [...offsets]
      .map((offset) => new Date(targetLocal - offset))
      .filter(
        (candidate) =>
          comparableMinute(zonedParts(candidate, timezone)) === targetLocal,
      )
      .sort((left, right) => left.getTime() - right.getTime());
    if (matches[0]) {
      return matches[0];
    }
  }
  throw new Error(`Could not resolve ${dateKey} ${time} in ${timezone}`);
}

function occurrence(
  meal: MealSchedule,
  dateKey: string,
  timezone: string,
): MealOccurrence {
  return {
    mealId: meal.id,
    label: meal.label,
    dateKey,
    scheduledAt: instantForLocalTime(
      dateKey,
      meal.time,
      timezone,
    ).toISOString(),
  };
}

export function nextMealOccurrence(
  meals: ReadonlyArray<MealSchedule>,
  now: Date,
  timezone: string,
): MealOccurrence | undefined {
  const enabled = meals.filter((meal) => meal.enabled);
  if (enabled.length === 0) {
    return undefined;
  }

  const today = dateKeyAt(now, timezone);
  for (let dayOffset = 0; dayOffset <= 7; dayOffset += 1) {
    const dateKey = addCalendarDays(today, dayOffset);
    const candidates = enabled
      .map((meal) => occurrence(meal, dateKey, timezone))
      .filter((candidate) => Date.parse(candidate.scheduledAt) >= now.getTime())
      .sort((left, right) => {
        const timeDifference =
          Date.parse(left.scheduledAt) - Date.parse(right.scheduledAt);
        return timeDifference || left.mealId.localeCompare(right.mealId);
      });
    if (candidates[0]) {
      return candidates[0];
    }
  }
  return undefined;
}

export function findDueMeal(
  meals: ReadonlyArray<MealSchedule>,
  now: Date,
  timezone: string,
  graceMinutes = 90,
): MealOccurrence | undefined {
  const today = dateKeyAt(now, timezone);
  const earliest = now.getTime() - Math.max(0, graceMinutes) * MINUTE_MS;
  const candidates = [addCalendarDays(today, -1), today]
    .flatMap((dateKey) =>
      meals
        .filter((meal) => meal.enabled)
        .map((meal) => occurrence(meal, dateKey, timezone)),
    )
    .filter((candidate) => {
      const scheduledAt = Date.parse(candidate.scheduledAt);
      return scheduledAt >= earliest && scheduledAt <= now.getTime();
    })
    .sort(
      (left, right) =>
        Date.parse(right.scheduledAt) - Date.parse(left.scheduledAt),
    );

  return candidates[0];
}

export function promptKey(
  occurrenceValue: Pick<MealOccurrence, "dateKey" | "mealId">,
): string {
  return `${occurrenceValue.dateKey}:${occurrenceValue.mealId}`;
}

export function dueLabel(dueAt: Date, now: Date, timezone: string): DueLabel {
  const differenceMs = dueAt.getTime() - now.getTime();
  if (Math.abs(differenceMs) < MINUTE_MS) {
    return { kind: "now" };
  }
  if (differenceMs < 0) {
    return {
      kind: "overdue",
      minutes: Math.max(1, Math.floor(-differenceMs / MINUTE_MS)),
    };
  }

  const dueParts = zonedParts(dueAt, timezone);
  const dueDateKey = dateKeyFromParts(dueParts);
  const today = dateKeyAt(now, timezone);
  const dueTime = `${padded(dueParts.hour)}:${padded(dueParts.minute)}`;
  if (dueDateKey === addCalendarDays(today, 1)) {
    return { kind: "tomorrow", time: dueTime };
  }
  if (dueDateKey !== today) {
    return { kind: "on_date", dateKey: dueDateKey, time: dueTime };
  }

  const minutes = Math.ceil(differenceMs / MINUTE_MS);
  if (minutes < 60) {
    return { kind: "in_minutes", minutes };
  }
  return { kind: "in_hours", hours: Math.round((minutes / 60) * 10) / 10 };
}

export const CLOCK_CONSTANTS = { MINUTE_MS, DAY_MS } as const;
