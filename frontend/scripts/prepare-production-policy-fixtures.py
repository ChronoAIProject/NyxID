#!/usr/bin/env python3
"""Prepare public test transport fixtures and verify the reviewed byte hashes.

These are test inputs, not build dependencies. A changed upstream response fails
closed; update the receipt only after reviewing the new vendor implementation.
"""
import argparse
import hashlib
import json
from pathlib import Path
from urllib.request import urlopen

FIXTURES = {
    "facebook-sdk.js": ("https://connect.facebook.net/en_US/sdk.js", "79def755066df6140389df062b7eb4487e7dbb4426fa7884dda2667f257593be"),
    "facebook-sdk-bundle.js": ("https://connect.facebook.net/en_US/bundle/sdk.js/", "e4a6792e49aef8983d55ca92c1d9b4c41809dd8e93905a3a280cf4db7ad8c314"),
    "telegram-widget-v22.js": ("https://telegram.org/js/telegram-widget.js?22", "3c8c27bf6f51a778ed94f452cf6b501f54703b67ef162551f677171916fbcd99"),
    "unicornstudio-v2.1.4.js": ("https://cdn.jsdelivr.net/gh/hiunicornstudio/unicornstudio.js@v2.1.4/dist/unicornStudio.umd.js", "28892da7fbec4f7704f335936672d4fce6f43483f3dc506beba7543d0c3ec4d1"),
    "unicorn-scene.json": ("https://assets.unicorn.studio/embeds/sqyz3r4WjVjdLal6IHvt", "706cbe09a17ca5fa2d5a3d562cf7c41a8c374241b3d76daf72f4405798da8e9d"),
    "jetbrainsmono-v24.ttf": ("https://fonts.gstatic.com/s/jetbrainsmono/v24/tDbY2o-flEEny0FZhsfKu5WU4zr3E_BX0PnT8RD8yKxjPQ.ttf", "44ce4a84f20d60f24539bd0cef11f79c29e38609e0f8adf18551c9794a5d9dc3"),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    recorded = Path(__file__).parents[1] / "e2e-production/fixtures"
    receipts = []
    for name, (source, expected) in FIXTURES.items():
        target = args.output / name
        if target.exists():
            data = target.read_bytes()
        elif (recorded / name).exists():
            data = (recorded / name).read_bytes()
        else:
            with urlopen(source, timeout=30) as response:
                data = response.read(2 * 1024 * 1024 + 1)
        digest = hashlib.sha256(data).hexdigest()
        if len(data) > 2 * 1024 * 1024 or digest != expected:
            raise ValueError(f"Fixture {name} changed: review upstream bytes before changing the receipt")
        target.write_bytes(data)
        receipts.append({"file": name, "source": source, "size": len(data), "sha256": digest})
    (args.output / "sources.json").write_text(json.dumps(receipts, indent=2) + "\n")
    print(json.dumps({"verified_fixtures": len(receipts), "directory": str(args.output.resolve())}))


if __name__ == "__main__":
    main()
