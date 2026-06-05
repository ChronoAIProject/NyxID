import { bootCredentialAcceptPage } from "./app";

const root = document.getElementById("credential-accept-root");

if (root instanceof HTMLElement) {
  bootCredentialAcceptPage(root);
}
