"""Execute the actual packaged binaries without the build shell's library paths.

Releases carry no ROM, so this checks what a player meets first: both
executables report the release version; the app, started from another
working directory with no ROM, fails with the instructions and makes the
empty rom/ folder beside itself (beside the .app on macOS); the
headless runner reports the missing ROM. `--rom` (a private ROM) also runs
the game in both.

`android` installs the APK on a fresh headless emulator (the android
shell's x86_64 system image, with KVM): it reports the release version,
asks for a ROM, makes its rom/ folder, shows the system's document picker
and quits when the player does; its arm64-v8a library does the same
through the image's ARM translation. With `--rom` the ROM is chosen in the
picker and the game plays 600 frames; a second start finds the copy the app
kept and plays on while the device turns to landscape and back; and the
arm64-v8a library plays too. Logs and screenshots are kept in
build/android/smoke/.
"""

import argparse
import contextlib
import os
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import time
import zipfile
from pathlib import Path
from xml.etree import ElementTree

ROOT = Path(__file__).resolve().parents[2]
APP = "Pokémon Card GB2.app"
PACKAGE = "io.github.pokemon_card_game_2_native_port"
ACTIVITY = f"{PACKAGE}/.GameActivity"
# The app's folder in the emulator's shared storage.
FILES = f"/sdcard/Android/data/{PACKAGE}/files"
SERIAL = "emulator-5584"


def adb(*arguments, check=True, timeout=120):
    return subprocess.run(
        ["adb", "-s", SERIAL, *map(str, arguments)], capture_output=True, text=True, check=check, timeout=timeout
    )


def boot_emulator(out, env):
    """A fresh emulator, booted: wiped, without a window, drawn with SwiftShader."""
    image = f"system-images;android-{os.environ['TCG2_ANDROID_PLATFORM']};google_apis;x86_64"
    subprocess.run(
        ["avdmanager", "create", "avd", "--force", "--name", "tcg2-smoke", "--package", image, "--device", "pixel_6"],
        input="no\n",
        capture_output=True,
        text=True,
        check=True,
        env=env,
    )
    log = (out / "emulator.log").open("w")
    emulator = subprocess.Popen(
        ["emulator", "-avd", "tcg2-smoke", "-port", SERIAL.split("-")[1], "-no-window", "-no-audio"]
        + ["-no-boot-anim", "-no-snapshot", "-wipe-data", "-no-metrics", "-gpu", "swiftshader_indirect"],
        stdout=log,
        stderr=subprocess.STDOUT,
        env=env,
    )
    deadline = time.monotonic() + 900
    while adb("shell", "getprop", "sys.boot_completed", check=False).stdout.strip() != "1":
        if emulator.poll() is not None or time.monotonic() > deadline:
            raise SystemExit(f"the emulator did not boot (see {out / 'emulator.log'})")
        time.sleep(3)
    # Right after boot the system still reconfigures apps, which would
    # recreate the app's activity: let it settle. No animations, and no
    # first-time hint over the full-screen app.
    time.sleep(20)
    for namespace, name, value in [
        ("global", "window_animation_scale", 0),
        ("global", "transition_animation_scale", 0),
        ("global", "animator_duration_scale", 0),
        ("secure", "immersive_mode_confirmations", "confirmed"),
        ("system", "accelerometer_rotation", 0),
        ("system", "user_rotation", 0),
    ]:
        adb("shell", "settings", "put", namespace, name, value)
    return emulator


# The app's own lines, SDL's, and those of a crash: Google's services on the
# system image log far more than all of these.
LOG_TAGS = ["SDL/APP", "SDL", "AndroidRuntime:E", "libc:F", "DEBUG:F"]


def logged(pattern, out, timeout=120):
    """Waits for the app (or SDL) to log a line matching `pattern`; returns it."""
    deadline = time.monotonic() + timeout
    while True:
        # A read can fail while the system logs heavily: the next one is
        # tried a second later.
        log = adb("logcat", "-d", "-v", "brief", "-s", *LOG_TAGS, check=False).stdout
        (out / "logcat.txt").write_text(log)
        if match := re.search(pattern, log):
            return match[0]
        # A Java exception, a Rust panic or a signal on SDL's thread.
        crashed = re.search(rf"Process: {re.escape(PACKAGE)}|pokemon-tcg2: panic|Fatal signal.*\(SDLThread\)", log)
        if crashed:
            raise SystemExit(f"the app crashed (see {out / 'logcat.txt'})")
        if time.monotonic() > deadline:
            raise SystemExit(f"nothing logged matching {pattern!r} (see {out / 'logcat.txt'})")
        time.sleep(1)


def tap(pattern, out, timeout=30):
    """Taps the element on screen whose text or description matches `pattern`."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if tap_now(pattern, out):
            return
        time.sleep(1)
    raise SystemExit(f"nothing on screen matching {pattern!r} (see {out / 'ui.xml'})")


def tap_now(pattern, out):
    # Outside shared storage, where the picker would list it.
    adb("shell", "uiautomator", "dump", "/data/local/tmp/ui.xml", check=False)
    xml = adb("exec-out", "cat", "/data/local/tmp/ui.xml").stdout
    (out / "ui.xml").write_text(xml)
    try:
        nodes = ElementTree.fromstring(xml).iter("node")
    except ElementTree.ParseError:
        return False
    for node in nodes:
        if any(re.fullmatch(pattern, node.get(name, ""), re.IGNORECASE) for name in ["text", "content-desc"]):
            x1, y1, x2, y2 = map(int, re.findall(r"\d+", node.get("bounds")))
            adb("shell", "input", "tap", (x1 + x2) // 2, (y1 + y2) // 2)
            return True
    return False


def focused():
    """The window with the focus, as the window manager reports it."""
    windows = adb("shell", "dumpsys", "window").stdout
    return next((line for line in windows.splitlines() if "mCurrentFocus=" in line), "")


def wait_for_picker(out, timeout=60):
    deadline = time.monotonic() + timeout
    while "documentsui" not in focused():
        if time.monotonic() > deadline:
            raise SystemExit(f"no document picker: {focused()}")
        time.sleep(1)
    screenshot(out / "picker.png")


def pick_document(name, out, timeout=120):
    """Chooses the document `name` in the picker, from its Downloads."""
    wait_for_picker(out)
    deadline = time.monotonic() + timeout
    while "documentsui" in focused():
        if time.monotonic() > deadline:
            raise SystemExit(f"{name} not chosen in the picker (see {out / 'ui.xml'})")
        for pattern in [re.escape(name), "Downloads", "Show roots"]:
            if tap_now(pattern, out):
                break
        time.sleep(2)


def screenshot(path):
    path.write_bytes(subprocess.check_output(["adb", "-s", SERIAL, "exec-out", "screencap", "-p"], timeout=60))


def start(*arguments):
    """Starts the app afresh, with the desktop's command-line `arguments`."""
    adb("shell", "am", "force-stop", PACKAGE)
    adb("logcat", "-c")
    extra = ["--esa", "args", ",".join(map(str, arguments))] if arguments else []
    adb("shell", "am", "start", "-W", "-n", ACTIVITY, *extra)


def ask_and_quit(out, abi):
    """Without a ROM the app asks for one, shows the picker, and quits when told."""
    start()
    logged(r"pokemon-tcg2: no ROM in \S+: asking for one", out)
    adb("shell", "test", "-d", f"{FILES}/rom")
    screenshot(out / f"ask-{abi}.png")
    tap("Choose ROM", out)
    wait_for_picker(out)
    adb("shell", "input", "keyevent", "KEYCODE_BACK")
    tap("Quit", out)
    logged(r"pokemon-tcg2: no ROM chosen: quitting", out)
    logged(r"Finished main function", out)


def play(frames, out, name):
    """Plays `frames` frames, leaving a screenshot in the app's folder; returns the app's --stats line."""
    picture = f"{FILES}/{name}.png"
    adb("shell", "rm", "-f", picture)
    start("--exit-after-frames", frames, "--screenshot", picture, "--stats")
    return play_on(picture, out, name)


def play_on(picture, out, name, timeout=600):
    stats = logged(r"pokemon-tcg2: \d+ frames in .*", out, timeout)
    adb("pull", picture, out / f"{name}.png")
    with (out / f"{name}.png").open("rb") as png:
        if png.read(8) != b"\x89PNG\r\n\x1a\n":
            raise SystemExit(f"no picture from the game: {picture}")
    return stats


def android(args):
    apk = ROOT / "dist" / f"pokemon-tcg2-{args.version}-android.apk"
    if not apk.is_file():
        raise SystemExit(f"missing {apk}")
    out = ROOT / "build/android/smoke"
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    home = ROOT / "build/android/home"
    env = dict(
        os.environ, ANDROID_USER_HOME=str(home), ANDROID_EMULATOR_HOME=str(home), ANDROID_AVD_HOME=str(home / "avd")
    )
    emulator = boot_emulator(out, env)
    try:
        adb("install", apk, timeout=300)
        if f"versionName={args.version[1:]}" not in adb("shell", "dumpsys", "package", PACKAGE).stdout:
            raise SystemExit("the installed APK has the wrong version")
        ask_and_quit(out, "x86_64")
        if args.rom:
            adb("push", args.rom, "/sdcard/Download/poketcg2.gbc")
            # The first start imports the ROM through the picker and plays.
            picture = f"{FILES}/import.png"
            start("--exit-after-frames", 600, "--screenshot", picture, "--stats")
            tap("Choose ROM", out)
            pick_document("poketcg2.gbc", out)
            logged(r"pokemon-tcg2: ROM \S+ kept as \S+", out)
            print("x86_64, ROM imported:", play_on(picture, out, "import"))
            adb("shell", "test", "-f", f"{FILES}/rom/poketcg2.gbc")
            adb("shell", "test", "-f", f"{FILES}/saves/poketcg2.sav")
            # The next start finds the copy without asking. The device
            # turns to landscape and back while the game plays.
            picture = f"{FILES}/x86_64.png"
            start("--exit-after-frames", 1200, "--screenshot", picture, "--stats")
            time.sleep(5)
            screenshot(out / "portrait.png")
            for rotation, name in [(1, "landscape"), (0, None)]:
                adb("shell", "settings", "put", "system", "user_rotation", rotation)
                time.sleep(4)
                if name:
                    screenshot(out / f"{name}.png")
            print("x86_64:", play_on(picture, out, "x86_64"))
            if "asking for one" in (out / "logcat.txt").read_text():
                raise SystemExit("the app asked for its ROM again")
        abis = adb("shell", "getprop", "ro.product.cpu.abilist").stdout
        if "arm64-v8a" not in abis:
            raise SystemExit(f"the emulator cannot run arm64-v8a: {abis.strip()}")
        adb("install", "-r", "--abi", "arm64-v8a", apk, timeout=300)
        if "primaryCpuAbi=arm64-v8a" not in adb("shell", "dumpsys", "package", PACKAGE).stdout:
            raise SystemExit("the arm64-v8a library was not installed")
        if args.rom:
            print("arm64-v8a, translated:", play(120, out, "arm64-v8a"))
        else:
            adb("shell", "rm", "-rf", f"{FILES}/rom")
            ask_and_quit(out, "arm64-v8a")
    except BaseException:
        # What the device showed and logged, if it still answers.
        with contextlib.suppress(Exception):
            (out / "logcat-full.txt").write_text(adb("logcat", "-d", "-v", "time", check=False, timeout=60).stdout)
            screenshot(out / "failure.png")
        raise
    finally:
        adb("emu", "kill", check=False)
        try:
            emulator.wait(timeout=120)
        except subprocess.TimeoutExpired:
            emulator.kill()
        subprocess.run(["adb", "kill-server"], check=False)
    played = ", imported the ROM and played" if args.rom else ""
    print(f"{apk.name}: installed, asked for a ROM, showed the picker and quit{played}, on x86_64 and arm64-v8a")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=["linux", "windows", "macos", "android"])
    parser.add_argument("--version", required=True)
    parser.add_argument(
        "--graphical", action="store_true", help="test the Linux GUI under Xvfb with SDL's software renderer"
    )
    parser.add_argument("--rom", type=Path, help="also run the game with a private ROM")
    args = parser.parse_args()
    if args.kind == "android":
        return android(args)
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
