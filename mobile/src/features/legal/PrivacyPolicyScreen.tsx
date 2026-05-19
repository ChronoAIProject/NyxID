import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { RootStackParamList } from "../../app/AppNavigator";
import { LegalDocumentScreen } from "./LegalDocumentScreen";

type Props = NativeStackScreenProps<RootStackParamList, "PrivacyPolicy">;

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
