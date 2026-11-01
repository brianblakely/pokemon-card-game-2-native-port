"""The authored source of one commit as a reproducible archive.

The port's own files only: src/decomp is pret's disassembly (published at
https://github.com/pret/poketcg2 and in this repository) and holds the
game's extracted assets, which releases never redistribute.
"""

import argparse
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--commit", default="HEAD")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    prefix = args.output.name.removesuffix(".tar.gz") + "/"
    with args.output.open("wb") as archive:
        subprocess.run(
            [
                "git",
                "-C",
                str(ROOT),
                "archive",
                "--format=tar.gz",
                f"--prefix={prefix}",
                args.commit,
                "--",
                ".",
                ":(exclude)src/decomp",
            ],
            stdout=archive,
            check=True,
        )
    print(args.output)


if __name__ == "__main__":
    main()
