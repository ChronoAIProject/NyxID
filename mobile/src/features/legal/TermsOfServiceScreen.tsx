import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { RootStackParamList } from "../../app/AppNavigator";
import { LegalDocumentScreen } from "./LegalDocumentScreen";

type Props = NativeStackScreenProps<RootStackParamList, "TermsOfService">;

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
