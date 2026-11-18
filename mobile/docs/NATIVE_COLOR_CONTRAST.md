# Native zinc theme and contrast

Neutral surfaces and text follow `pr-2064-ref:frontend/src/app.css`: dark
canvas #17171A, sidebar #1C1C1F, card #222225; light canvas #F4F4F5,
sidebar/card #FFFFFF. Native drawers retain their surface and top-edge semantics.
Hairlines use the reference white 14% / black 20%; selected fills stay neutral.

Native accessibility adaptations use the same purple hue with distinct values:
accent text is #C6B3FC in dark mode (the web's primary-text blend) and #7049C6
in light mode; tinted text uses #C4B5FD / #6D45B5. The original #9775FA on white
was about 3.36:1 and cannot serve normal-size text. White CTA labels use a
#8752CC → #5E00F5 gradient; the web's #A672FB start was about 3.27:1.
Tests sample the entire native gradient, including its pressed stops.

Essential shared field/button boundaries use `controlBorder` (#80808A dark,
#85858D light), tested at 3:1 on every base/drawer surface. Decorative card
borders, composer outlines, scrims and hairlines intentionally have lower
contrast; filled controls also have a contrasting icon and a 44pt target.
Disabled shared action buttons retain semantic tertiary text and an explicit
accessibility disabled state. Their labels are tested at 4.5:1 despite the WCAG
inactive-control exemption.

`mobileTheme.test.ts` checks text, selected and interactive fills, tinted
status/error labels, gradients, submit glyphs/circles, control edges and the
drawer against composited chat. This is token-level verification. Legacy
screens still use opacity on some disabled controls, and per-component pressed
opacity can reduce contrast. Those states require separate runtime review;
the token tests do not certify every rendered pixel or every native OS control.
