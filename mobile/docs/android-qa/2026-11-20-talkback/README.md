# Android TalkBack follow-up for #2651

Base: `upstream/main` at `b6d194f99`. Verified on 2026-11-20.

The closed sidebar edge on this base is a `Pressable` with `accessible=false`,
but Android still reports it as `focusable=true` and important for accessibility.
On a cold start with TalkBack enabled it receives the initial accessibility
focus without a name. This branch explicitly removes the closed edge from both
native focus and accessibility traversal. The named open-menu Close action
retains both.

## Native validation

- AOSP Android API 34 ARM64 emulator, `NyxID_QA_API34_B`, 1080 × 2400.
- Real TalkBack 16.2, package `com.android.talkback`, enabled with touch
  exploration. Binary from the GrapheneOS TalkBack prebuilt repository; verified
  against Git blob `d3bae02cf5cefecd62c0f57e7e76d78808439cb6` and the GrapheneOS
  signing certificate. This is distinct from the Xiaomi QA device's Google
  Android Accessibility Suite build.
- Expo Go SDK 55 host with this branch's actual `WorkspacePushShell`, actual
  native-stack header/Menu control, and `WorkspaceInteractionSurface`. The body
  contains named fixture controls for Home, Activity, Agent list, Scan QR, New
  chat and Message; it does not sign in or mutate an account.
- A standalone `UiAutomation` instrumentation probe read
  `root.findFocus(AccessibilityNodeInfo.FOCUS_ACCESSIBILITY)` after each gesture.
  It connected with `FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES` and never chose
  the next target or sent an accessibility-focus action. TalkBack remained
  enabled throughout the run.

The base focused `workspace-sidebar-reveal-edge`; the hardware gesture baseline
also visited that unnamed edge before advancing to Open navigation. With the fix, the
same native node has `focusable=false`, `important=false` and no accessibility
focus. Physical touchscreen events then traversed:

`Open navigation → Home → Home title → Activity → Agent list → Scan QR → New chat → Message`

These were real changes to the platform's accessibility focus, not assertions
about React props. The complete sequence (including the Expo Go Tools control
and launch settling) is in [focus-results.json](focus-results.json). Screenshots
show the focus on [Open navigation](fixed-open-navigation.png),
[Home title](fixed-home-title.png), and [Message](fixed-message.png).
The [baseline screenshot](baseline-hidden-edge.png) shows the unnamed edge.

### Gesture input matters

`adb shell input swipe 300 1000 850 1000 180` left accessibility focus unchanged,
including on Open navigation after this fix. This synthetic motion did **not**
advance TalkBack. Its unchanged focus must not be treated as a successful
TalkBack next-item gesture or evidence of a remaining trap by itself.

The successful run sent touchscreen hardware events through the emulator
console, which enter the kernel/InputReader path used by touch exploration:

```python
for x in range(300, 851, 55):
    subprocess.run([
        adb, "-s", "emulator-5572", "emu", "event", "mouse",
        str(x), "1000", "0", "1",
    ], check=True)
    time.sleep(0.012)
subprocess.run([
    adb, "-s", "emulator-5572", "emu", "event", "mouse",
    "850", "1000", "0", "0",
], check=True)
```

Both the base and fixed shell advanced beyond Open navigation with hardware
touchscreen events. The confirmed source defect is the unwanted unnamed focus
stop; the reported 2026-11-20 functional trap on the named Open navigation control
was not reproduced with these events. This does not establish which input path
the original QA used. Retesting the complete app on the reported Xiaomi/MIUI
device remains necessary to close that platform-specific report conclusively.

## Focused checks

From `mobile/`:

```sh
pnpm exec tsx --test src/components/workspace/WorkspacePushShell.test.ts src/lib/accessibility/workspaceFocusState.test.ts src/lib/accessibility/WorkspaceInteractionSurface.test.ts
pnpm exec jest --findRelatedTests src/components/workspace/WorkspacePushShell.tsx --runInBand --passWithNoTests
```

The first command passed all 40 tests. The Jest dependency query found no
matching Jest tests; the directly corresponding node tests ran explicitly.
Changed-file whitespace validation, `git diff --check`, passed. No affected
typecheck target or mobile lint command exists. The full frontend suite,
typecheck and production build are delegated to GitHub CI.

The emulator's accessibility service setting is restored to its initial disabled
state after validation. No real account data or credentials appear in artifacts.
