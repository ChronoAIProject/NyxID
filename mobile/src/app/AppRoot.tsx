import App from "./App";
import { DomainGate } from "./DomainGate";

export default function AppRoot() {
  return (
    <DomainGate>
      <App />
    </DomainGate>
  );
}
