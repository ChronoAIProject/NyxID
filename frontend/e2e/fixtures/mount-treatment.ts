import { createRoot } from "react-dom/client";
import { createElement } from "react";
import { TreatmentFixture } from "./treatment-parity";

/** Retain each real route's shell and portal context while comparing identical content. */
export function mountTreatment() {
  const parent = document.querySelector(".shell-page-content")!;
  const host = document.createElement("div");
  host.dataset.treatmentHost = "";
  parent.prepend(host);
  createRoot(host).render(createElement(TreatmentFixture));
}
