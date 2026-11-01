"""Check native release inputs and reserve a version; retries reuse the same tag."""

import argparse
import json
import os
import re
import subprocess


def native_changed(base, commit):
    """Compare the whole push, not just its last commit or its commit messages.

    These are the native app's game, host and build inputs. Tests,
    documentation, test tooling and publication machinery do not change it.
    """
    if base == "0" * 40:
        base = subprocess.check_output(["git", "hash-object", "-t", "tree", "--stdin"], input="", text=True).strip()
    result = subprocess.run(
        [
            "git",
            "diff",
            "--quiet",
            base,
            commit,
            "--",
            "src/decomp/",
            "src/runtime/crates/",
            "src/runtime/Cargo.toml",
            "src/runtime/Cargo.lock",
            "src/runtime/android/",
            "nix/runtime/",
            "packaging/install.txt",
            "tools/release/package.py",
            "tools/release/shell.sh",
            ":(exclude)src/runtime/crates/tcg2-reference/",
            ":(glob,exclude)**/tests/**",
            ":(glob,exclude)**/examples/**",
            ":(glob,exclude)**/benches/**",
            ":(glob,exclude)**/*.md",
        ],
        check=False,
    )
    if result.returncode not in (0, 1):
        result.check_returncode()
    return result.returncode == 1


def version(tag):
    match = re.fullmatch(r"v(\d+)\.(\d+)(?:\.(\d+))?", tag)
    return tuple(int(n or 0) for n in match.groups()) if match else None


def next_tag(tags):
    versions = [number for tag in tags if (number := version(tag)) is not None]
    if not versions:
        return "v0.1"
    major, minor, patch = max(max(versions), (0, 1, 0))
    return f"v{major}.{minor}.{patch + 1}"


def gh(*arguments, data=None):
    command = ["gh", "api", *arguments]
    if data is not None:
        command += ["--input", "-"]
    return subprocess.run(command, input=json.dumps(data) if data is not None else None, text=True, capture_output=True)


def reserve(repository, commit, preview=False):
    for _ in range(10):
        response = gh(f"repos/{repository}/git/matching-refs/tags/v")
        response.check_returncode()
        refs = json.loads(response.stdout)
        tags = {item["ref"].removeprefix("refs/tags/"): item["object"] for item in refs}
        for tag, obj in tags.items():
            if version(tag) is not None and obj["type"] == "commit" and obj["sha"] == commit:
                return tag
        tag = next_tag(tags)
        if preview:
            return tag
        response = gh(
            f"repos/{repository}/git/refs", "--method", "POST", data={"ref": f"refs/tags/{tag}", "sha": commit}
        )
        if response.returncode == 0:
            return tag
        if '"Reference already exists"' not in response.stdout:
            raise RuntimeError(response.stderr + response.stdout)
    raise RuntimeError("concurrent releases repeatedly reserved the next version")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository")
    parser.add_argument("--commit", required=True)
    parser.add_argument("--preview", action="store_true", help="build a qualification branch without creating a tag")
    parser.add_argument("--check-native-since", metavar="BASE", help="only check inputs; never reserve a tag")
    args = parser.parse_args()
    if args.check_native_since:
        changed = native_changed(args.check_native_since, args.commit)
        values = f"changed={str(changed).lower()}\n"
    else:
        if not args.repository:
            parser.error("--repository is required to reserve a version")
        values = f"tag={reserve(args.repository, args.commit, args.preview)}\n"
    print(values, end="")
    if path := os.environ.get("GITHUB_OUTPUT"):
        with open(path, "a") as output:
            output.write(values)


if __name__ == "__main__":
    main()
