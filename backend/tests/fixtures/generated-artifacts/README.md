# Generated-artifact media fixtures

These are synthetic, non-secret fixtures for issue #2037. They are shared by the
bounded Sandbox parser tests and the isolated Chromium attachment tests. Browser
tests use fixture HTTP responses; they do not constitute live Sandbox acceptance.

| File | Verified properties | SHA-256 |
| --- | --- | --- |
| `visual-avc.mp4` | H.264 Constrained Baseline, 160 × 96, yuv420p, 1 second; real visual sample tables and AVC configuration; Chromium play/pause/download fixture | `9cd06e9521279eb15b0d6a6bffa7ac48ed49ed80b7f286bf6b4de3d269f788a2` |
| `visual-high.mp4` | H.264 High, 160 × 96, yuv420p, 1 second; valid container, download-only under the current preview allowlist | `176d7742d9d5d6cc094ca90201caf9a1399427b02db08017f730cd1e7b2c5ff5` |
| `blue.png` | Real 16 × 16 PNG | `385a92647193b6011c5e5e5260d7b7014351c56cabb30add4d458ae6efc49557` |
| `blue.jpg` | Real 16 × 16 JPEG | `094f19af980673362f9a2539ae3b2efa6cc42eec7e038c89f840322c97de1d0f` |

Inspect an MP4 without decoding arbitrary untrusted files:

```sh
ffprobe -v error -select_streams v:0 \
  -show_entries stream=codec_name,profile,width,height,pix_fmt,duration \
  -of json backend/tests/fixtures/generated-artifacts/visual-avc.mp4
```

The High fixture was generated with:

```sh
ffmpeg -hide_banner -loglevel error -f lavfi -i color=c=blue:s=160x96:r=12 \
  -t 1 -c:v libx264 -profile:v high -pix_fmt yuv420p -movflags +faststart \
  backend/tests/fixtures/generated-artifacts/visual-high.mp4
```

For an equivalent Baseline fixture, use `testsrc2=s=160x96:r=12` with
`-profile:v baseline -pix_fmt yuv420p -movflags +faststart`. Encoder versions can
change bytes; regeneration requires updating the digest and rerunning parser
and real-browser tests. A codec name or marker string alone never enables preview.
