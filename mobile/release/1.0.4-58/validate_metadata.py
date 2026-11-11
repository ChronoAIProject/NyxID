"""Check the App Store field limits for the build 58 English submission copy."""

from pathlib import Path


ROOT = Path(__file__).resolve().parent
LIMITS = {
    "en-US/name.txt": 30,
    "en-US/subtitle.txt": 30,
    "en-US/promotional-text.txt": 170,
    "en-US/keywords.txt": 100,
    "en-US/description.txt": 4000,
    "en-US/whats-new.txt": 4000,
    "app-review-notes.txt": 4000,
    "testflight-testing-notes.txt": 4000,
    "resubmission-message.txt": 4000,
}


def main() -> None:
    failures = []
    for relative, limit in LIMITS.items():
        value = (ROOT / relative).read_text(encoding="utf-8").rstrip("\n")
        count = len(value)
        print(f"{relative}: {count}/{limit}")
        if not value.strip() or count > limit:
            failures.append(relative)
    if failures:
        raise SystemExit("Invalid submission fields: " + ", ".join(failures))


if __name__ == "__main__":
    main()
