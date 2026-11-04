import controlMp4 from "@/assets/onboarding/control.mp4";
import controlWebm from "@/assets/onboarding/control.webm";
import controlPoster from "@/assets/onboarding/control.webp";
import chiefMp4 from "@/assets/onboarding/chief-of-staff.mp4";
import chiefWebm from "@/assets/onboarding/chief-of-staff.webm";
import chiefPoster from "@/assets/onboarding/chief-of-staff.webp";
import specialistsMp4 from "@/assets/onboarding/specialists.mp4";
import specialistsWebm from "@/assets/onboarding/specialists.webm";
import specialistsPoster from "@/assets/onboarding/specialists.webp";

/** The recorded intro replays: VP9, an H.264 fallback and a poster frame each. */
export const CLIPS = {
  "chief-of-staff": { webm: chiefWebm, mp4: chiefMp4, poster: chiefPoster },
  specialists: { webm: specialistsWebm, mp4: specialistsMp4, poster: specialistsPoster },
  control: { webm: controlWebm, mp4: controlMp4, poster: controlPoster },
} as const;

let preloaded = false;

/**
 * Warm the cache with every replay while the intro's first screens are read,
 * so each one shows (and plays) the moment its step opens.
 */
export function preloadReplays() {
  if (preloaded || typeof document === "undefined") return;
  preloaded = true;
  const webm = document.createElement("video").canPlayType('video/webm; codecs="vp9"') !== "";
  for (const files of Object.values(CLIPS)) {
    new Image().src = files.poster;
    void fetch(webm ? files.webm : files.mp4).catch(() => undefined);
  }
}
