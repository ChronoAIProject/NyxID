import { websiteConfiguration } from "../website-a/release-config";
/** Refuse mixed public HTML/JS builds before enabling any interactive controls. */
export function assertPublicConfiguration(
  root: HTMLElement,
  configuration = websiteConfiguration,
) {
  if (
    root.dataset.publicRender === "v1" &&
    root.getAttribute("data-website-config") !==
      encodeURIComponent(configuration)
  )
    throw new Error(
      "Public website configuration mismatch; reload the complete release.",
    );
}
