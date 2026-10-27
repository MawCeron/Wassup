#!/usr/bin/env python3
"""Writes ui/src/window/third_party.rs: the licenses of the code Wassup bundles,
grouped by license, for the About dialog.

It reads the dependencies that end up in each binary (Rust crates with
`cargo tree`, Go modules with `go list -deps`), looks for their license files
in the Cargo registry and the Go module cache, and adds the emoji font and
SQLite, which come with no crate or module of their own. Run it again, and
commit the result, when the dependencies change:

    cargo fetch --manifest-path ui/Cargo.toml && (cd daemon && go mod download)
    packaging/third-party.py
"""
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "ui/src/window/third_party.rs")
TARGET = "x86_64-unknown-linux-gnu"
# When a crate offers a choice of licenses, the first of these is the one used.
PREFERENCE = ["MIT", "Apache-2.0", "Zlib", "BSD-3-Clause", "BSD-2-Clause", "ISC", "Unlicense"]
LICENSE_FILE = re.compile(r"(?i)^(licen[sc]e|copying|unlicense)")
SQLITE_BLESSING = """The author disclaims copyright to this source code. In place of a legal notice, here is a blessing:

   May you do good and not evil.
   May you find forgiveness for yourself and forgive others.
   May you share freely, never taking more than you give."""


def run(cmd, cwd):
    return subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, text=True).stdout


def detect(text):
    """The SPDX id of a license text, or None."""
    t = " ".join(text.split())
    if "Mozilla Public License" in t and "2.0" in t:
        return "MPL-2.0"
    if "GNU GENERAL PUBLIC LICENSE" in t and "Version 3" in t:
        return "GPL-3.0-only"
    if "Apache License" in t and "Version 2.0" in t:
        return "Apache-2.0"
    if "Permission is hereby granted, free of charge" in t:
        return "MIT"
    if re.search(r"Permission to use, copy, modify, (and/or|and) distribute this software", t):
        return "ISC"
    if "Redistribution and use in source and binary forms" in t:
        return "BSD-3-Clause" if "Neither the name" in t else "BSD-2-Clause"
    if "This is free and unencumbered software released into the public domain" in t:
        return "Unlicense"
    if "zlib" in t.lower() and "altered source versions must be plainly marked" in t:
        return "Zlib"
    return None


def choose(expression):
    """The licenses a crate is used under, from its SPDX expression: every AND
    part applies, and an OR part is one of its options."""
    ids = []
    for part in re.split(r"\s+AND\s+", expression):
        options = [re.sub(r"\s+WITH\s+.*", "", o).strip("() ") for o in re.split(r"\s+OR\s+", part.strip("() "))]
        options = [o.replace("MIT/Apache-2.0", "MIT") for o in options]
        picked = next((p for p in PREFERENCE if p in options), options[0])
        ids.append(picked)
    return ids


def license_files(directory):
    names = sorted(n for n in os.listdir(directory) if LICENSE_FILE.match(n) and os.path.isfile(os.path.join(directory, n)))
    return [(n, open(os.path.join(directory, n), encoding="utf-8", errors="replace").read()) for n in names]


def pick_file(files, license_id):
    """The license file of a package that holds the text of `license_id`."""
    for name, text in files:
        if detect(text) == license_id:
            return text
    for name, text in files:
        if license_id.lower().split("-")[0] in name.lower():
            return text
    return files[0][1] if files else None


def copyright_lines(text):
    lines = []
    for line in text.splitlines():
        s = line.strip().lstrip("*#/ ").strip()
        if re.match(r"^(Copyright|\(c\)|©)", s) and re.search(r"\b(19|20)\d\d\b", s) and not re.search(r"\[yyyy\]|\{yyyy\}|notice|holder|owner|<year>", s, re.I):
            lines.append(s)
    return lines


sections = {}  # id -> {"packages": [...], "notices": set, "text": str|None}


def add(license_id, package, text, notices=None):
    s = sections.setdefault(license_id, {"packages": [], "notices": [], "texts": []})
    s["packages"].append(package)
    for n in (notices if notices is not None else (copyright_lines(text) if text else [])):
        if n not in s["notices"]:
            s["notices"].append(n)
    if text:
        s["texts"].append(text)


def most_common_text(texts):
    """The text most packages share once copyright lines are left out: the standard one."""
    counts = {}
    for t in texts:
        key = " ".join(without_copyright(t).split())
        counts.setdefault(key, [0, t])[0] += 1
    return max(counts.values(), key=lambda c: c[0])[1] if counts else ""


def without_copyright(text):
    """The text of a license with its copyright lines taken out: they are listed apart."""
    kept = [l for l in text.splitlines() if l.strip() not in copyright_lines(l)]
    return "\n".join(kept).strip("\n")


# Rust: the crates in the binary.
metadata = json.loads(run(["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET], os.path.join(ROOT, "ui")))
manifests = {(p["name"], p["version"]): os.path.dirname(p["manifest_path"]) for p in metadata["packages"]}
tree = run(["cargo", "tree", "-e", "normal", "--target", TARGET, "--prefix", "none", "--format", "{p}|{l}"], os.path.join(ROOT, "ui"))
crates = {}
for line in tree.splitlines():
    line = line.replace(" (*)", "")
    name_version, _, expression = line.partition("|")
    if "(" in name_version.split("|")[0] and "/" in name_version:  # the workspace crate, from a path
        continue
    name, version = name_version.split()[0], name_version.split()[1].lstrip("v")
    if name == "wassup":
        continue
    crates[(name, version)] = expression
for (name, version), expression in sorted(crates.items()):
    files = license_files(manifests[(name, version)])
    for license_id in choose(expression):
        text = pick_file(files, license_id)
        if text is None:
            print(f"warning: no license file for {name} {version}", file=sys.stderr)
        add(license_id, f"{name} {version}", text)

# Go: the modules in the daemon.
modules = run(["go", "list", "-deps", "-f", "{{if .Module}}{{if not .Module.Main}}{{.Module.Path}}|{{.Module.Version}}|{{.Module.Dir}}{{end}}{{end}}", "./..."], os.path.join(ROOT, "daemon"))
for entry in sorted(set(modules.split())):
    path, version, directory = entry.split("|")
    files = license_files(directory)
    ids = {detect(t) for _, t in files} - {None}
    if not ids:
        print(f"warning: unknown license for {path} {version}", file=sys.stderr)
        continue
    for license_id in sorted(ids):
        if path == "go.mau.fi/libsignal":
            license_id = "GPL-3.0-only"
        add(license_id, f"{path} {version}", pick_file(files, license_id))

# Not in a crate or a module of their own.
add("SQLite public domain dedication", "SQLite (in modernc.org/sqlite)", None, [])
sections["SQLite public domain dedication"]["texts"] = [SQLITE_BLESSING]
font_notice = open(os.path.join(ROOT, "data/licenses/twemoji-cbdt-NOTICE"), encoding="utf-8").read().strip()
font_license = open(os.path.join(ROOT, "data/licenses/twemoji-cbdt-LICENSE-GRAPHICS"), encoding="utf-8").read().strip()
add("CC-BY-4.0", "Twemoji CBDT (the emoji font)", font_license, [])
sections["CC-BY-4.0"]["notices"] = [font_notice]

# The Rust file.
DELIMITER = '"####'


def raw(text):
    assert DELIMITER not in text
    return 'r####"' + text + DELIMITER


with open(OUT, "w", encoding="utf-8") as f:
    f.write("//! The licenses of the code Wassup bundles, by license, for the About dialog.\n")
    f.write("//! Generated by packaging/third-party.py; do not edit.\n\n")
    f.write("pub(super) struct Section {\n    pub title: &'static str,\n    pub packages: &'static str,\n    pub notices: &'static str,\n    pub text: &'static str,\n}\n\n")
    f.write("pub(super) const SECTIONS: &[Section] = &[\n")
    for license_id in sorted(sections):
        s = sections[license_id]
        text = most_common_text(s["texts"])
        body = without_copyright(text) if license_id not in ("CC-BY-4.0", "SQLite public domain dedication") and text else text
        f.write("    Section {\n")
        f.write(f"        title: {raw(license_id)},\n")
        f.write(f"        packages: {raw(', '.join(sorted(set(s['packages']))))},\n")
        f.write(f"        notices: {raw(chr(10).join(s['notices']))},\n")
        f.write(f"        text: {raw(body)},\n")
        f.write("    },\n")
    f.write("];\n")

print(f"wrote {os.path.relpath(OUT, ROOT)}: {len(sections)} licenses, "
      f"{sum(len(set(s['packages'])) for s in sections.values())} packages", file=sys.stderr)
for license_id in sorted(sections, key=lambda i: -len(sections[i]["packages"])):
    print(f"  {license_id}: {len(set(sections[license_id]['packages']))} packages", file=sys.stderr)
