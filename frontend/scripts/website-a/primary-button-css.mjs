// Paint each primary gradient across its transparent border without tiling.
export function fixPrimaryButtonPaint(sheet) {
  sheet.walkDecls("background", (declaration) => {
    if (
      declaration.value === "var(--vivid)" &&
      declaration.parent.type === "rule" &&
      declaration.parent.selectors.some((selector) =>
        /\.btn\.p\b/.test(selector),
      )
    ) {
      declaration.value = "var(--vivid) border-box no-repeat";
    }
  });
}
