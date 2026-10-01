// Public limits mirrored from models/assistant_settings.rs. Keep protocol
// fallbacks centralized; saved settings from the server always take precedence.
export const DEFAULT_SCHEDULE_MINIMUM_MINUTES = 5;
export const SCHEDULE_MINIMUM_MINUTES_LIMIT = 1440;
export const DEFAULT_TRIGGER_RUNS_PER_HOUR = 30;
export const TRIGGER_RUNS_PER_HOUR_LIMIT = 300;
export const DEFAULT_TRIGGER_RUNS_PER_DAY = 300;
export const TRIGGER_RUNS_PER_DAY_LIMIT = 3000;
