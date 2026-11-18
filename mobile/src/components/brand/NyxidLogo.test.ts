import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import * as vectors from "./brandVectors";

test("NyxID vector logo picks the resolved theme at the web sidebar height", () => {
  for (const mode of ["dark", "light"]) {
    const harness = hookHarness<any>({
      "react/jsx-runtime": {
        jsx: (type: unknown, props: unknown) => ({ type, props }),
      },
      "react-native": { View: "View" },
      "react-native-svg": { SvgXml: "SvgXml" },
      "../../theme/ThemeContext": { useTheme: () => ({ mode }) },
      "./brandVectors": vectors,
    });
    const { NyxidLogo } = harness.load(
      fileURLToPath(new URL("./NyxidLogo.tsx", import.meta.url)),
    ) as { NyxidLogo: (props: object) => any };
    const logo = NyxidLogo({});
    assert.equal(logo.props.accessibilityLabel, "NyxID");
    assert.equal(logo.props.style.height, 20);
    assert.equal(logo.props.style.width, (20 * 1062) / 301);
    const child = logo.props.children;
    assert.equal(child.type, "SvgXml");
    assert.equal(child.props.accessible, false);
    assert.equal(
      child.props.xml,
      readFileSync(
        new URL(
          `../../../assets/brand/nyxid-coloured-logo${mode === "light" ? "-dark" : ""}.svg`,
          import.meta.url,
        ),
        "utf8",
      ),
    );
  }
});
