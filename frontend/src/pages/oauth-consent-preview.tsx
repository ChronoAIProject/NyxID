import { OAuthConsentPage } from "./oauth-consent";
import { previewSearch, previewServices } from "./oauth-consent-preview-data";

export function OAuthConsentPreviewPage() {
  return (
    <OAuthConsentPage
      preview={{
        search: previewSearch,
        services: previewServices,
        email: "alex@example.com",
      }}
    />
  );
}
