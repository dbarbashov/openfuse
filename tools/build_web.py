#!/usr/bin/env python3
"""Build the browser editor from the shared Rust implementation."""
import argparse
import json
import package_release
import pathlib
import shutil
import subprocess
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]

def build(output):
    version = next(p["version"] for p in tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
                   if p["name"] == "wasm-bindgen")
    installed = subprocess.check_output(["wasm-bindgen", "--version"], text=True).strip()
    if installed != f"wasm-bindgen {version}":
        raise SystemExit(f"Install the locked CLI: cargo install wasm-bindgen-cli --version {version} --locked")
    build_result = subprocess.run(["cargo", "build", "--release", "--locked", "-p", "openfuse-desktop", "--lib",
                    "--target", "wasm32-unknown-unknown", "--message-format=json"],
                    cwd=ROOT, check=True, stdout=subprocess.PIPE, text=True)
    output.mkdir(parents=True, exist_ok=True)
    for name in ("index.html", "app.js"):
        shutil.copyfile(ROOT / "web" / name, output / name)
    subprocess.run(["wasm-bindgen", "--target", "web", "--no-typescript", "--out-dir", str(output / "pkg"),
                    str(ROOT / "target/wasm32-unknown-unknown/release/openfuse_desktop.wasm")], check=True)
    shutil.copyfile(ROOT / "packaging/openfuse.svg", output / "openfuse.svg")
    for name in ("LICENSE", "THIRD_PARTY.md", "DISCLAIMER.md"):
        shutil.copyfile(ROOT / name, output / name)
    built = {item["package_id"] for line in build_result.stdout.splitlines()
             if line.startswith("{") and (item := json.loads(line)).get("reason") == "compiler-artifact"}
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1",
                           "--filter-platform", "wasm32-unknown-unknown"], cwd=ROOT, text=True))
    notices = output / "third-party"
    if notices.exists():
        shutil.rmtree(notices)
    report = package_release.dependency_notices(metadata, notices, built)
    if report["without_license_or_supplement"]:
        raise SystemExit(f"Missing dependency notices: {report['without_license_or_supplement']}")
    print(f"Browser editor: {output}")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, default=ROOT / "target/web")
    build(parser.parse_args().output.resolve())
