"""Build portable release artifacts inside the pinned Nix release shells.

The kernel comes from `build/kernel` (tcg2-recomp's translation of the
decomp, made once by the release's prepare job), so platforms need no RGBDS.
Packages hold the app, its read-me and notices: never game data.

Linux: SDL3 is compiled into the app, linked against an old glibc with zig;
the system's own graphics, sound and input libraries are opened at run time,
so nothing else is shipped (and no runtime sources are owed). Windows and
macOS carry SDL3 (and its toolchain libraries) beside the app.

Android: one APK for arm64-v8a and x86_64. The app is built as a library
with the NDK, beside libSDL3.so from the same SDL source as the Linux
release's (the sdl3-src crate, zlib-licensed, as is its Java activity), and
the SDK's tools make the APK directly: nothing is downloaded at build time,
and the same inputs and key make the same file.
"""

import argparse
import gzip
import hashlib
import json
import os
import platform
import plistlib
import re
import shutil
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools.release.runtime_sources import bundle as bundle_sources  # noqa: E402
from tools.release.runtime_sources import store_root  # noqa: E402

RUNTIME = ROOT / "src/runtime"
DIST = ROOT / "dist"
APP = "Pokémon Card GB2.app"
# SDL3 runs from Android 5 (API 21); AAudio, Android's low-latency sound
# path, is there for all from Android 8 (API 26). The target is the
# android shell's SDK platform (TCG2_ANDROID_PLATFORM).
ANDROID_MIN_SDK = 26
ANDROID_ABIS = {"arm64-v8a": "aarch64-linux-android", "x86_64": "x86_64-linux-android"}


def run(*command, **kwargs):
    return subprocess.run([str(p) for p in command], check=True, **kwargs)


def zip_tree(source, destination):
    """Only the staged files, stable timestamps and Unix executable modes."""
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sorted(source.rglob("*")):
            if path.is_symlink():
                raise ValueError(f"unexpected staged symlink: {path}")
            if path.is_file():
                info = zipfile.ZipInfo(path.relative_to(source).as_posix(), (1980, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.external_attr = (path.stat().st_mode & 0xFFFF) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, path.read_bytes())


def tar_tree(source, destination):
    """The staged folder as one top-level directory, with stable metadata."""

    def normalise(info):
        info.uid = info.gid = 0
        info.uname = info.gname = ""
        info.mtime = 0
        return info

    with open(destination, "wb") as raw:
        with gzip.GzipFile(fileobj=raw, mode="wb", compresslevel=9, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                archive.add(source, arcname=source.name, recursive=False, filter=normalise)
                for path in sorted(source.rglob("*")):
                    if path.is_symlink():
                        raise ValueError(f"unexpected staged symlink: {path}")
                    archive.add(
                        path,
                        arcname=f"{source.name}/{path.relative_to(source).as_posix()}",
                        recursive=False,
                        filter=normalise,
                    )


def notices(stage):
    shutil.copyfile(ROOT / "packaging/install.txt", stage / "README.txt")
    shutil.copyfile(ROOT / "build/THIRD_PARTY_NOTICES.txt", stage / "THIRD_PARTY_NOTICES.txt")


def rom_folder(stage):
    """The empty folder the player's ROM goes in, with a note saying so."""
    (stage / "rom").mkdir()
    shutil.copyfile(ROOT / "packaging/rom-readme.txt", stage / "rom/README.txt")


def windows_libraries(stage):
    # Query the actual toolchain/SDL closure; never assume a developer's DLLs
    # are installed on the player's machine.
    providers = json.loads(os.environ["TCG2_WINDOWS_LIBRARIES"])
    closure = subprocess.check_output(["nix-store", "-qR", *providers], text=True).splitlines()
    available = {p.name.lower(): p for root in closure for p in Path(root).rglob("*.dll")}
    # Windows system DLLs. Everything else must be shipped or fail the build.
    system = {
        "kernel32.dll",
        "user32.dll",
        "gdi32.dll",
        "advapi32.dll",
        "shell32.dll",
        "ole32.dll",
        "oleaut32.dll",
        "ws2_32.dll",
        "ntdll.dll",
        "msvcrt.dll",
        "ucrtbase.dll",
        "bcrypt.dll",
        "userenv.dll",
        "secur32.dll",
        "shlwapi.dll",
        "imm32.dll",
        "setupapi.dll",
        "version.dll",
        "winmm.dll",
        "cfgmgr32.dll",
        "dwmapi.dll",
        "comdlg32.dll",
        "winspool.drv",
        "dxgi.dll",
        "d3d12.dll",
        "d3d11.dll",
        "dxguid.dll",
        "hid.dll",
        "crypt32.dll",
        "propsys.dll",
        "dbghelp.dll",
        "bcryptprimitives.dll",
        "dwrite.dll",
        "combase.dll",
        "opengl32.dll",
    }
    pending = list(stage.glob("*.exe"))
    copied = set()
    # Include the toolchain's statically linked startup/thread support too.
    components = set(providers)
    while pending:
        binary = pending.pop()
        output = subprocess.check_output(["x86_64-w64-mingw32-objdump", "-p", str(binary)], text=True)
        for name in re.findall(r"DLL Name:\s*(\S+)", output):
            key = name.lower()
            if key in system or key.startswith(("api-ms-win-", "ext-ms-win-")) or key in copied:
                continue
            if key not in available:
                raise ValueError(f"unbundled Windows dependency: {name} ({binary.name})")
            target = stage / name
            shutil.copyfile(available[key], target)
            copied.add(key)
            components.add(store_root(available[key]))
            pending.append(target)
    return components


def macos_libraries(bundle):
    frameworks = bundle / "Contents/Frameworks"
    frameworks.mkdir()
    pending = list((bundle / "Contents/MacOS").iterdir())
    copied = {}
    while pending:
        binary = pending.pop()
        binary.chmod(0o755)
        output = subprocess.check_output(["otool", "-L", str(binary)], text=True)
        for line in output.splitlines()[1:]:
            library = line.strip().split(" (", 1)[0]
            if library.startswith(("/System/Library/", "/usr/lib/", "@")):
                continue
            source = Path(library)
            if not source.is_file():
                raise ValueError(f"unbundled macOS dependency: {library}")
            if source.name in copied and copied[source.name] != source:
                raise ValueError(f"two libraries named {source.name}")
            target = frameworks / source.name
            if source.name not in copied:
                copied[source.name] = source
                shutil.copyfile(source, target)
                target.chmod(0o755)
                run("install_name_tool", "-id", f"@rpath/{source.name}", target)
                pending.append(target)
            relative = "@loader_path/" if binary.parent == frameworks else "@executable_path/../Frameworks/"
            run("install_name_tool", "-change", library, relative + source.name, binary)
    # Ad-hoc signatures keep modified Mach-O load commands valid on Apple
    # Silicon. This is not an Apple Developer ID/notarization claim.
    for library in frameworks.iterdir():
        run("codesign", "--force", "--sign", "-", library)
    run("codesign", "--force", "--deep", "--sign", "-", bundle)
    return {store_root(path) for path in copied.values()}


def android_version_code(version):
    """Android's whole number for a version, which an update must raise: v0.1 is 10000, v0.1.2 is 10002."""
    major, minor, patch = (int(n or 0) for n in re.fullmatch(r"v(\d+)\.(\d+)(?:\.(\d+))?", version).groups())
    if minor > 99 or patch > 9999:
        raise ValueError(f"no Android version code for {version}")
    return major * 1_000_000 + minor * 10_000 + patch


def sdl_source():
    """SDL's source as Cargo.lock pins the sdl3-src crate: libSDL3.so is built from it, and its Java side must match."""
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--locked", "--format-version", "1", "--all-features"], cwd=RUNTIME, text=True
        )
    )
    (package,) = [package for package in metadata["packages"] if package["name"] == "sdl3-src"]
    return Path(package["manifest_path"]).parent / "SDL"


def android_key():
    """The signing key and the environment holding its password.

    Releases sign with the project's key (TCG2_ANDROID_KEYSTORE, a PKCS12
    keystore holding one RSA key, its password in
    TCG2_ANDROID_KEYSTORE_PASSWORD); an installed APK only takes updates
    signed with the key it came with. Without it, a development key made
    once in build/android/ signs instead, for testing.
    """
    if keystore := os.environ.get("TCG2_ANDROID_KEYSTORE"):
        if "TCG2_ANDROID_KEYSTORE_PASSWORD" not in os.environ:
            raise SystemExit("TCG2_ANDROID_KEYSTORE needs TCG2_ANDROID_KEYSTORE_PASSWORD")
        return Path(keystore), os.environ
    keystore = ROOT / "build/android/development.keystore"
    env = dict(os.environ, TCG2_ANDROID_KEYSTORE_PASSWORD="development")
    if not keystore.is_file():
        keystore.parent.mkdir(parents=True, exist_ok=True)
        # RSA signatures are deterministic: a rebuild signs to the same bytes.
        run(
            "keytool",
            "-genkeypair",
            "-keystore",
            keystore,
            "-storetype",
            "PKCS12",
            "-storepass",
            "development",
            "-alias",
            "development",
            "-keyalg",
            "RSA",
            "-keysize",
            "3072",
            "-validity",
            "10000",
            "-dname",
            "CN=Pokemon Card GB2 native port development",
        )
    print(f"signing with the development key {keystore} (not for publishing)")
    return keystore, env


def android(stage, artifact, version, env, target):
    """The APK: SDL's activity, and for each ABI the app's library and libSDL3.so; aligned and signed."""
    sdk = Path(os.environ["ANDROID_HOME"])
    ndk = Path(os.environ["ANDROID_NDK_HOME"])
    tools = Path(os.environ["TCG2_ANDROID_BUILD_TOOLS"])
    platform_version = os.environ["TCG2_ANDROID_PLATFORM"]
    android_jar = sdk / f"platforms/android-{platform_version}/android.jar"
    llvm = ndk / "toolchains/llvm/prebuilt/linux-x86_64/bin"
    for abi, triple in ANDROID_ABIS.items():
        key = triple.replace("-", "_")
        clang = llvm / f"{triple}{ANDROID_MIN_SDK}-clang"
        abi_env = dict(env)
        abi_env.update(
            {
                f"CC_{key}": str(clang),
                f"CXX_{key}": f"{clang}++",
                f"AR_{key}": str(llvm / "llvm-ar"),
                f"CARGO_TARGET_{key.upper()}_LINKER": str(clang),
                # sdl3-sys builds libSDL3.so with CMake, through the NDK.
                f"CMAKE_TOOLCHAIN_FILE_{key}": str(ndk / "build/cmake/android.toolchain.cmake"),
            }
        )
        command = ["cargo", "rustc", "--locked", "--release", "-p", "tcg2-desktop", "--lib", "--crate-type", "cdylib"]
        run(*command, "--features", "shared-sdl", "--target", triple, cwd=RUNTIME, env=abi_env)
        built = target / triple / "release"
        libraries = stage / "lib" / abi
        libraries.mkdir(parents=True)
        # SDL's activity loads libSDL3.so, then libmain.so for SDL_main.
        shutil.copyfile(built / "libtcg2_desktop.so", libraries / "libmain.so")
        run(llvm / "llvm-strip", "--strip-unneeded", "-o", libraries / "libSDL3.so", built / "libSDL3.so")
        for library in libraries.iterdir():
            # Android 15 runs on devices with 16 KB pages too.
            headers = subprocess.check_output([llvm / "llvm-readelf", "--program-headers", library], text=True)
            aligns = [int(line.split()[-1], 16) for line in headers.splitlines() if line.split()[:1] == ["LOAD"]]
            if not aligns or min(aligns) < 0x4000:
                raise ValueError(f"{library} is not aligned for 16 KB pages: {aligns}")
    # SDL's Java side, and the activity that passes the app its arguments.
    classes = stage / "classes"
    sources = sorted((sdl_source() / "android-project/app/src/main/java/org/libsdl/app").glob("*.java"))
    javac = ["javac", "-source", "8", "-target", "8", "-bootclasspath", android_jar, "-encoding", "UTF-8"]
    run(*javac, "-nowarn", "-Xlint:-options", "-d", classes, *sources, ROOT / "src/runtime/android/GameActivity.java")
    dex = ["--release", "--min-api", ANDROID_MIN_SDK, "--lib", android_jar, "--output", stage]
    run(tools / "d8", *dex, *sorted(classes.rglob("*.class")))
    linked = stage / "linked.apk"
    run(
        tools / "aapt2",
        "link",
        "-o",
        linked,
        "-I",
        android_jar,
        "--manifest",
        ROOT / "src/runtime/android/AndroidManifest.xml",
        "--min-sdk-version",
        ANDROID_MIN_SDK,
        "--target-sdk-version",
        platform_version,
        "--version-code",
        android_version_code(version),
        "--version-name",
        version[1:],
    )
    # One archive with stable metadata, everything compressed (the
    # libraries to a quarter): Android unpacks the libraries when it
    # installs the app.
    unaligned = stage / "unaligned.apk"
    with zipfile.ZipFile(linked) as manifest, zipfile.ZipFile(unaligned, "w") as apk:

        def add(name, data, compress):
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED if compress else zipfile.ZIP_STORED
            apk.writestr(info, data, compresslevel=9)

        for item in manifest.infolist():
            add(item.filename, manifest.read(item), item.compress_type != zipfile.ZIP_STORED)
        add("classes.dex", (stage / "classes.dex").read_bytes(), True)
        for library in sorted((stage / "lib").rglob("*.so")):
            add(library.relative_to(stage).as_posix(), library.read_bytes(), True)
        add("assets/THIRD_PARTY_NOTICES.txt", (ROOT / "build/THIRD_PARTY_NOTICES.txt").read_bytes(), True)
    aligned = stage / "aligned.apk"
    run(tools / "zipalign", "-f", "4", unaligned, aligned)
    keystore, key_env = android_key()
    password = "env:TCG2_ANDROID_KEYSTORE_PASSWORD"
    # Signature schemes 2 and 3, all Android 8 needs.
    schemes = ["--v1-signing-enabled", "false", "--v4-signing-enabled", "false"]
    sign = ["sign", "--ks", keystore, "--ks-pass", password, *schemes, "--out", artifact]
    run(tools / "apksigner", *sign, aligned, env=key_env)
    run(tools / "apksigner", "verify", "--min-sdk-version", ANDROID_MIN_SDK, artifact)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=["linux", "windows", "macos", "android"])
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"v\d+\.\d+(?:\.\d+)?", args.version):
        parser.error("expected a version such as v0.1 or v0.1.1")
    kernel = ROOT / "build/kernel"
    if not (kernel / "kernel.rs").is_file():
        parser.error(f"missing {kernel / 'kernel.rs'}; translate the decomp first (tcg2-recomp --out build/kernel)")
    DIST.mkdir(exist_ok=True)
    arch = "x86_64" if args.kind == "windows" else platform.machine().replace("arm64", "aarch64")
    # One APK holds every Android ABI.
    stem = f"pokemon-tcg2-{args.version}-{args.kind}" + ("" if args.kind == "android" else f"-{arch}")
    stage = ROOT / "build/release" / stem
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    env = dict(
        os.environ,
        TCG2_RELEASE_VERSION=args.version,
        TCG2_KERNEL_DIR=str(kernel),
        CARGO_PROFILE_RELEASE_STRIP="symbols",
    )
    target = Path(os.environ.get("CARGO_TARGET_DIR", RUNTIME / "target-release")).resolve()
    env["CARGO_TARGET_DIR"] = str(target)
    if args.kind == "android":
        artifact = DIST / f"{stem}.apk"
        android(stage, artifact, args.version, env, target)
        print(f"{hashlib.sha256(artifact.read_bytes()).hexdigest()}  {artifact.name}")
        return
    command = ["cargo", "build", "--locked", "--release", "-p", "tcg2-desktop", "-p", "tcg2-headless"]
    if args.kind == "windows":
        command += ["--target", "x86_64-pc-windows-gnu"]
        target /= "x86_64-pc-windows-gnu"
    if args.kind == "linux":
        # Runs on every distribution with glibc 2.31 (2020) or newer.
        triple = f"{arch}-unknown-linux-gnu"
        command[1] = "zigbuild"
        command += ["--features", "tcg2-desktop/static-sdl", "--target", f"{triple}.{os.environ['TCG2_LINUX_GLIBC']}"]
        target /= triple
    run(*command, cwd=RUNTIME, env=env)
    binaries = stage
    if args.kind == "macos":
        bundle = stage / APP
        binaries = bundle / "Contents/MacOS"
        binaries.mkdir(parents=True)
        (bundle / "Contents/Info.plist").write_bytes(
            plistlib.dumps(
                {
                    "CFBundleExecutable": "pokemon-tcg2",
                    "CFBundleIdentifier": "io.github.pokemon-card-game-2-native-port",
                    "CFBundleName": "Pokémon Card GB2",
                    "CFBundlePackageType": "APPL",
                    "CFBundleShortVersionString": args.version[1:],
                    "CFBundleVersion": args.version[1:],
                    "NSHighResolutionCapable": True,
                    "LSMinimumSystemVersion": "13.0",
                    "LSApplicationCategoryType": "public.app-category.games",
                }
            )
        )
    for name in ["pokemon-tcg2", "tcg2-headless"]:
        name += ".exe" if args.kind == "windows" else ""
        shutil.copy2(target / "release" / name, binaries / name)
    notices(stage)
    rom_folder(stage)
    if args.kind == "linux":
        artifact = DIST / f"{stem}.tar.gz"
        tar_tree(stage, artifact)
    else:
        components = windows_libraries(stage) if args.kind == "windows" else macos_libraries(bundle)
        artifact = DIST / f"{stem}.zip"
        zip_tree(stage, artifact)
        bundle_sources(sorted(components), [os.environ["TCG2_NIXPKGS_SOURCE"]], DIST / f"{stem}-runtime-sources.tar.gz")
    print(f"{hashlib.sha256(artifact.read_bytes()).hexdigest()}  {artifact.name}")


if __name__ == "__main__":
    main()
