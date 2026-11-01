"""Generate dependency notices; unknown licenses or missing text block release."""

import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def main():
    output = ROOT / "build/THIRD_PARTY_NOTICES.txt"
    output.parent.mkdir(exist_ok=True)
    result = subprocess.run(
        [
            "cargo-about",
            "generate",
            "--locked",
            "--manifest-path",
            str(ROOT / "src/runtime/Cargo.toml"),
            "--config",
            str(ROOT / "packaging/about.toml"),
            str(ROOT / "packaging/about.hbs"),
            "--output-file",
            str(output),
        ],
        capture_output=True,
        text=True,
    )
    if result.returncode or "has no license file" in result.stderr:
        raise SystemExit(result.stdout + result.stderr)
    with output.open("a") as notices:
        notices.write("\n\nSDL 3\n\n" + (Path(os.environ["TCG2_SDL3_SRC"]) / "LICENSE.txt").read_text())
    print(output)


if __name__ == "__main__":
    main()
