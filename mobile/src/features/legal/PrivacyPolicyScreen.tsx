import type { SheetScreenProps } from "../../app/navigationTypes";
import { LegalDocumentScreen } from "./LegalDocumentScreen";

type Props = SheetScreenProps<"PrivacyPolicy">;

export function PrivacyPolicyScreen({ navigation }: Props) {
  return (
    <LegalDocumentScreen
      title="Privacy Policy"
      docKey="privacy"
      telemetryBackTarget="privacy_back"
      onBack={() => navigation.goBack()}
    />
  );
}
