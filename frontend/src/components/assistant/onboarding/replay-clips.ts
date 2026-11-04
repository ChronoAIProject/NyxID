import chiefStill from "@/assets/onboarding/chief-of-staff-still.webp";
import chief from "@/assets/onboarding/chief-of-staff.webp";
import controlStill from "@/assets/onboarding/control-still.webp";
import control from "@/assets/onboarding/control.webp";
import specialistsStill from "@/assets/onboarding/specialists-still.webp";
import specialists from "@/assets/onboarding/specialists.webp";

/**
 * The recorded intro replays as animated WebP (played by the image pipeline,
 * like a GIF: no autoplay rules or video decoder involved), each with a still
 * frame for reduced motion.
 */
export const CLIPS = {
  "chief-of-staff": { animated: chief, still: chiefStill },
  specialists: { animated: specialists, still: specialistsStill },
  control: { animated: control, still: controlStill },
} as const;

let preloaded = false;

/**
 * Warm the cache with every replay while the intro's first screens are read,
 * so each one shows (and plays) the moment its step opens.
 */
export function preloadReplays() {
  if (preloaded || typeof Image === "undefined") return;
  preloaded = true;
  for (const files of Object.values(CLIPS)) {
    new Image().src = files.animated;
    new Image().src = files.still;
  }
}
