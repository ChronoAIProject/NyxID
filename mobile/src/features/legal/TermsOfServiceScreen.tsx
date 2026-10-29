import type { SheetScreenProps } from "../../app/navigationTypes";
import { LegalDocumentScreen } from "./LegalDocumentScreen";

type Props = SheetScreenProps<"TermsOfService">;

export function TermsOfServiceScreen({ navigation }: Props) {
  return (
    <LegalDocumentScreen
      title="Terms of Service"
      docKey="terms"
      telemetryBackTarget="terms_back"
      onBack={() => navigation.goBack()}
    />
  );
}
