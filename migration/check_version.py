"""Assert that the Go version constant equals the Rust oracle's package version.

  python3 -I migration/check_version.py [REPO_ROOT]

`internal/core/version.go` must hold exactly one `const Version = "<version>"` line, and that
version must equal `[package].version` in `Cargo.toml`. Exit 0 when they are equal; otherwise
print the mismatch to stderr and exit 1.
"""
import os
import re
import sys
import tomllib

VERSION_LINE = re.compile(r'^const Version = "([^"]+)"$', re.MULTILINE)


def cargo_version(root):
    with open(os.path.join(root, "Cargo.toml"), "rb") as f:
        return tomllib.load(f)["package"]["version"]


def go_version(root):
    path = os.path.join(root, "internal", "core", "version.go")
    with open(path, encoding="utf-8") as f:
        matches = VERSION_LINE.findall(f.read())
    if len(matches) != 1:
        raise SystemExit(f"{path}: expected exactly one 'const Version = \"<version>\"' line, found {len(matches)}")
    return matches[0]


def main(argv):
    root = os.path.abspath(argv[1] if len(argv) > 1 else os.path.join(os.path.dirname(__file__), os.pardir))
    rust, go = cargo_version(root), go_version(root)
    if rust != go:
        print(f"version mismatch: Cargo.toml package version {rust!r} != internal/core/version.go Version {go!r}",
              file=sys.stderr)
        return 1
    print(f"versions match: {go}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
