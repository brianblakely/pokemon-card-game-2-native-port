"""Execute the actual packaged binaries without the build shell's library paths.

Releases carry no ROM, so this checks what a player meets first: both
executables report the release version; the app, started from another
working directory with no ROM, fails with the instructions and makes the
empty rom/ folder beside itself (beside the .app on macOS); the
headless runner reports the missing ROM. `--rom` (a private ROM) also runs
the game in both.
"""

import argparse
import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
APP = "Pokémon Card GB2.app"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=["linux", "windows", "macos"])
    parser.add_argument("--version", required=True)
    parser.add_argument(
        "--graphical", action="store_true", help="test the Linux GUI under Xvfb with SDL's software renderer"
    )
    parser.add_argument("--rom", type=Path, help="also run the game with a private ROM")
    args = parser.parse_args()
    arch = platform.machine().lower().replace("amd64", "x86_64").replace("arm64", "aarch64")
    stem = f"pokemon-tcg2-{args.version}-{args.kind}-{arch}"
    env = {k: v for k, v in os.environ.items() if not k.startswith(("TCG2_", "LD_", "DYLD_"))}
    env["TCG2_NO_DIALOG"] = "1"
    (ROOT / "build").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="tcg2 portable ", dir=ROOT / "build") as tmp:
        root = Path(tmp)
        elsewhere = root / "different working directory"
        elsewhere.mkdir()
        if args.kind == "linux":
            with tarfile.open(ROOT / "dist" / f"{stem}.tar.gz") as archive:
                archive.extractall(root, filter="data")
            root /= stem
            desktop, headless = [str(root / "pokemon-tcg2")], [str(root / "tcg2-headless")]
            # The app must make rom/ in its own folder when it is gone.
            shutil.rmtree(root / "rom")
        else:
            with zipfile.ZipFile(ROOT / "dist" / f"{stem}.zip") as archive:
                archive.extractall(root)
                if args.kind == "macos":
                    for item in archive.infolist():
                        (root / item.filename).chmod((item.external_attr >> 16) & 0o777)
            binaries = root if args.kind == "windows" else root / APP / "Contents/MacOS"
            suffix = ".exe" if args.kind == "windows" else ""
            desktop, headless = [str(binaries / f"pokemon-tcg2{suffix}")], [str(binaries / f"tcg2-headless{suffix}")]
            if args.kind == "windows":
                # The app must make rom/ in its own folder when it is gone.
                # (On macOS the folder beside the .app is used only when the
                # player keeps rom/ there, as the package arrives.)
                shutil.rmtree(root / "rom")
        for executable in [desktop, headless]:
            output = subprocess.check_output([*executable, "--version"], cwd=elsewhere, env=env, text=True, timeout=60)
            if args.version not in output:
                raise SystemExit(f"wrong packaged version: {output}")
        graphical = desktop
        if args.kind == "linux":
            env["SDL_VIDEO_DRIVER"] = "x11"
            env["SDL_AUDIO_DRIVER"] = "dummy"
            if args.graphical:
                # Xvfb has no GPU: the system's own X11 libraries, drawn by
                # SDL's software renderer.
                env["SDL_RENDER_DRIVER"] = "software"
                graphical = [shutil.which("xvfb-run"), "-a", *desktop]
        if args.kind != "linux" or args.graphical:
            missing = subprocess.run(graphical, cwd=elsewhere, env=env, capture_output=True, text=True, timeout=120)
            if missing.returncode == 0 or "No game ROM found" not in missing.stderr:
                raise SystemExit(f"no-ROM start did not explain itself: {missing.returncode} {missing.stderr}")
            if not (root / "rom").is_dir() or (elsewhere / "rom").exists():
                raise SystemExit("incorrect portable storage location: rom/")
        missing = subprocess.run(
            [*headless, "--frames", "1"], cwd=elsewhere, env=env, capture_output=True, text=True, timeout=60
        )
        if missing.returncode == 0 or "rom" not in missing.stderr.lower():
            raise SystemExit(f"headless start without a ROM did not fail cleanly: {missing.stderr}")
        if args.rom:
            (root / "rom").mkdir(exist_ok=True)
            shutil.copyfile(args.rom, root / "rom" / args.rom.name)
            subprocess.run(
                [*headless, "--rom", str(root / "rom" / args.rom.name), "--frames", "600"],
                cwd=elsewhere,
                env=env,
                check=True,
                timeout=120,
            )
            if args.kind != "linux" or args.graphical:
                subprocess.run(
                    [*graphical, "--exit-after-frames", "120", "--screenshot", str(root / "frame.png")],
                    cwd=elsewhere,
                    env=env,
                    check=True,
                    timeout=120,
                )
                if not (root / "frame.png").is_file() or not (root / "saves").is_dir():
                    raise SystemExit("packaged application produced no picture or saves/")
    print(f"{stem}: packaged startup passed")


if __name__ == "__main__":
    main()
