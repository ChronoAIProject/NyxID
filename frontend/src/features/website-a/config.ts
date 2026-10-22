import { resolveContactEmail } from "./contact-mailto";

const configuredEmail = import.meta.env.VITE_WEBSITE_CONTACT_EMAIL;
export const websiteConfig = {
  contactEmail: resolveContactEmail(configuredEmail),
};
if (
  configuredEmail?.trim() &&
  (/[\r\n]/.test(configuredEmail) ||
    websiteConfig.contactEmail !== configuredEmail.trim())
) {
  console.warn(
    "[nyxid-contact] Invalid mailbox configuration; default contact address retained.",
  );
}

export { websiteRelease, websiteVisibility } from "./release-config";
