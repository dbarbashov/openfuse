#!/usr/bin/env python3
"""Build an offline, native portable OpenFUSE package without installing it."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import time
import zipfile

ROOT = Path(__file__).resolve().parents[1]
GUIDES = ("usage.md",)
PROJECT_NOTICES = ("DISCLAIMER.md", "THIRD_PARTY.md")
MARKDOWN_LINK = re.compile(r"\[([^\]]+)\]\(([^)]+)\)")


def local_link(source, target):
    if "://" in target or target.startswith(("#", "mailto:")):
        return None
    return (source.parent / target.split("#", 1)[0]).resolve()


def allowed_guide(root, path):
    return path in {root / "docs/usage.md", root / "packaging/README.md",
                    root / "DISCLAIMER.md"}


def guide_files(root):
    """Copy only the explicit user-guide allowlist."""
    root = root.resolve()
    pending = [root / "docs" / filename for filename in GUIDES]
    pending.append(root / "packaging/README.md")
    pending.append(root / "DISCLAIMER.md")
    visited = set()
    while pending:
        source = pending.pop()
        if source in visited:
            continue
        if not source.is_file():
            raise ValueError(f"Missing packaged guide: {source}")
        visited.add(source)
        if source.suffix == ".md":
            for _, target in MARKDOWN_LINK.findall(source.read_text(encoding="utf-8")):
                path = local_link(source, target)
                if path is not None and allowed_guide(root, path):
                    pending.append(path)
    return sorted(visited)


def copy_guides(root, destination):
    root = root.resolve()
    files = guide_files(root)
    for source in files:
        target = destination / source.relative_to(root)
        target.parent.mkdir(parents=True, exist_ok=True)
        if source.suffix == ".md":
            def render(match):
                label, link = match.groups()
                path = local_link(source, link)
                if path is None or path in files:
                    return match.group(0)
                reference = path.relative_to(root).as_posix() if path.is_relative_to(root) else link
                return f"{label} (source repository reference: `{reference}`; not bundled)"
            target.write_text(MARKDOWN_LINK.sub(render, source.read_text(encoding="utf-8")),
                              encoding="utf-8")
        else:
            shutil.copyfile(source, target)
    return len(files)


def run(args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def source_hashes(root):
    paths = [root / "Cargo.toml", root / "Cargo.lock", root / "LICENSE",
             root / "tools/package_release.py"]
    paths.extend(root / name for name in PROJECT_NOTICES)
    paths.extend(guide_files(root))
    for folder in ("crates", "packaging"):
        paths.extend(path for path in (root / folder).rglob("*")
                     if path.is_file() and "__pycache__" not in path.parts)
    return {str(path.relative_to(root)): sha256(path) for path in sorted(paths)}


def app_info(version, minimum_os):
    return {
        "CFBundleName": "OpenFUSE", "CFBundleDisplayName": "OpenFUSE",
        "CFBundleIdentifier": "org.openfuse.editor", "CFBundleExecutable": "openfuse-desktop",
        "CFBundleIconFile": "openfuse.icns",
        "CFBundlePackageType": "APPL", "CFBundleShortVersionString": version,
        "CFBundleVersion": version, "NSHighResolutionCapable": True,
        "LSMinimumSystemVersion": minimum_os,
        "CFBundleDocumentTypes": [{"CFBundleTypeName": "Legacy FUSE preset",
            "CFBundleTypeRole": "Editor", "LSHandlerRank": "Alternate",
            "LSItemContentTypes": ["org.openfuse.preset"]}],
        "UTImportedTypeDeclarations": [{"UTTypeIdentifier": "org.openfuse.preset",
            "UTTypeDescription": "Legacy FUSE preset", "UTTypeConformsTo": ["public.xml"],
            "UTTypeTagSpecification": {"public.filename-extension": ["fuse", "FUSE"],
                                        "public.mime-type": "application/x-openfuse-preset"}}],
    }


def dependency_packages(metadata):
    """Resolve only the built CLI/desktop graph, excluding dev-only edges."""
    packages = {item["id"]: item for item in metadata["packages"]}
    nodes = {item["id"]: item for item in metadata["resolve"]["nodes"]}
    pending = [key for key, item in packages.items()
               if item["name"] in ("openfuse-cli", "openfuse-desktop") and item["source"] is None]
    visited = set()
    while pending:
        key = pending.pop()
        if key in visited:
            continue
        visited.add(key)
        for dep in nodes[key]["deps"]:
            if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]):
                pending.append(dep["pkg"])
    return sorted((packages[key] for key in visited if packages[key]["source"]),
                  key=lambda item: (item["name"], item["version"]))


def mac_icon(destination):
    """Render the original SVG using system tools; no downloaded artwork."""
    with tempfile.TemporaryDirectory(prefix="openfuse-icon-") as temp:
        iconset = Path(temp) / "openfuse.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                filename = f"icon_{size}x{size}" + ("@2x" if scale == 2 else "") + ".png"
                run(["sips", "-s", "format", "png", "-z", str(size * scale), str(size * scale),
                     str(ROOT / "packaging/openfuse.svg"), "--out", str(iconset / filename)],
                    stdout=subprocess.PIPE)
        run(["iconutil", "-c", "icns", str(iconset), "-o", str(destination)])


def dependency_notices(metadata, destination, built_packages=None):
    destination.mkdir(parents=True)
    sources = destination / "sources"
    sources.mkdir()
    supplement_root = ROOT / "packaging/third-party"
    supplement_file = supplement_root / "supplemental.json"
    supplements = (json.loads(supplement_file.read_text())["packages"]
                   if supplement_file.is_file() else {})
    inventory = []
    for package in dependency_packages(metadata):
        if built_packages is not None and package["id"] not in built_packages:
            continue
        name = package["name"] + "-" + package["version"]
        root = Path(package["manifest_path"]).parent
        if not package["source"].startswith("registry+"):
            raise ValueError(f"Unreviewed non-registry dependency: {name}")
        # Cargo's cache is adjacent to src/<registry>/<package>. Never download
        # a guessed artifact or substitute a different dependency version.
        archive = root.parents[2] / "cache" / root.parent.name / (name + ".crate")
        if not archive.is_file():
            raise ValueError(f"Missing exact Cargo source archive for {name}: {archive}")
        shutil.copyfile(archive, sources / archive.name)
        license_paths = []
        for path in sorted(root.rglob("*")):
            if path.is_file() and (path.name.upper().startswith(
                    ("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE"))
                    or path.parent.name == "fonts" and path.suffix == ".txt"):
                relative = path.relative_to(root)
                target = destination / "licenses" / name / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, target)
                license_paths.append(str(target.relative_to(destination)))
        supplement = supplements.get(name)
        supplemental_paths = []
        if supplement:
            if sha256(archive) != supplement["crate_sha256"]:
                raise ValueError(f"License supplement source mismatch: {name}")
            for item in supplement["files"]:
                relative = Path(item["file"])
                if relative.is_absolute() or ".." in relative.parts:
                    raise ValueError(f"License supplement path escape: {name}")
                source = (supplement_root / item["file"]).resolve()
                if not source.is_relative_to(supplement_root.resolve()):
                    raise ValueError(f"License supplement path escape: {name}")
                if sha256(source) != item["sha256"]:
                    raise ValueError(f"License supplement hash mismatch: {name}")
                target = destination / "licenses" / name / "supplemental" / item["file"]
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, target)
                supplemental_paths.append(str(target.relative_to(destination)))
        inventory.append({"name": package["name"], "version": package["version"],
            "license_expression": package["license"], "authors": package["authors"],
            "repository": package["repository"], "source_archive": "sources/" + archive.name,
            "source_sha256": sha256(archive), "license_files": license_paths,
            "supplemental_license_files": supplemental_paths,
            "supplement_provenance": supplement,
            "standalone_license_present": bool(license_paths)})
    write_json(destination / "inventory.json", inventory)
    (destination / "README.txt").write_text(
        "Third-party source and notices\n\n"
        "inventory.json records exact Cargo dependencies and license expressions.\n"
        "sources/*.crate are the unmodified published source archives (tar.gz).\n"
        "They include the corresponding MPL-2.0 Symphonia source code.\n"
        "licenses/ contains standalone license/notice files present in those crates.\n"
        "Reviewed supplemental notices are bound to exact crate and file hashes.\n"
        "Their provenance distinguishes upstream files from standard-text supplements.\n"
        "Some upstream crates omit standalone license files; original source and\n"
        "author metadata are retained without inventing upstream notices.\n"
        "This inventory is not a legal certification or a claim of license review.\n")
    return {"packages": len(inventory), "source_archives": len(inventory),
            "without_standalone_license": [item["name"] + "-" + item["version"]
                for item in inventory if not item["standalone_license_present"]],
            "without_license_or_supplement": [item["name"] + "-" + item["version"]
                for item in inventory if not item["standalone_license_present"]
                and not item["supplemental_license_files"]]}


def write_checksums(folder):
    items = [(sha256(path), path.relative_to(folder).as_posix())
             for path in sorted(folder.rglob("*"))
             if path.is_file() and not path.is_symlink() and path.name != "SHA256SUMS"]
    (folder / "SHA256SUMS").write_text("".join(f"{digest}  {name}\n" for digest, name in items))
    return len(items)


def make_archive(folder, destination, system):
    if system == "Windows":
        with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for path in sorted(folder.rglob("*")):
                if path.is_file():
                    archive.write(path, str(Path(folder.name) / path.relative_to(folder)))
    elif system == "Linux":
        with tarfile.open(destination, "w:gz") as archive:
            archive.add(folder, arcname=folder.name)
    elif system == "Darwin":
        run(["hdiutil", "create", "-volname", "OpenFUSE", "-srcfolder", str(folder),
             "-format", "UDZO", "-ov", str(destination)])
    else:
        raise ValueError("Unsupported native platform")


def verify_checksums(folder):
    count = 0
    for line in (folder / "SHA256SUMS").read_text().splitlines():
        digest, relative = line.split("  ", 1)
        path = folder / relative
        if path.is_symlink() or not path.resolve().is_relative_to(folder.resolve()):
            raise ValueError("Checksum path escapes package")
        if sha256(path) != digest:
            raise ValueError(f"Package checksum mismatch: {relative}")
        count += 1
    return count


def verify_mac_archive(archive):
    with tempfile.TemporaryDirectory(prefix="openfuse-package-check-") as temp:
        mount = Path(temp) / "volume"
        mount.mkdir()
        run(["hdiutil", "attach", "-readonly", "-nobrowse", "-mountpoint", str(mount),
             str(archive)], stdout=subprocess.PIPE)
        try:
            app = mount / "OpenFUSE.app"
            run(["codesign", "--verify", "--deep", "--strict", str(app)])
            count = verify_checksums(mount)
            for stem in ("openfuse-desktop", "openfuse"):
                run([str(app / "Contents/MacOS" / stem), "--help"], capture_output=True)
            return {"read_only_mount": "pass", "checksums_verified": count,
                    "ad_hoc_signature": "pass", "mounted_cli_help": "pass",
                    "mounted_desktop_help": "pass", "gui_launch": "not tested here"}
        finally:
            for attempt in range(5):
                result = subprocess.run(["hdiutil", "detach", str(mount)],
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                if result.returncode == 0:
                    break
                if attempt == 4:
                    raise RuntimeError(f"Cannot detach package verification volume: {mount}")
                time.sleep(2)


def package(output, universal=False):
    system = platform.system()
    if system not in ("Darwin", "Linux", "Windows"):
        raise ValueError("Supported packaging hosts: macOS, Linux, Windows")
    output = output.resolve()
    if output.exists():
        raise ValueError(f"Output already exists; use another --output directory: {output}")
    before = source_hashes(ROOT)
    build_env = os.environ.copy()
    minimum_os = build_env.get("MACOSX_DEPLOYMENT_TARGET", "11.0")
    if system == "Darwin":
        build_env["MACOSX_DEPLOYMENT_TARGET"] = minimum_os
    # Explicit host target prevents accidental ambient CARGO_BUILD_TARGET cross builds.
    rustc = run(["rustc", "-vV"], capture_output=True).stdout
    host = next(line.split(": ", 1)[1] for line in rustc.splitlines() if line.startswith("host:"))
    if universal and system != "Darwin":
        raise ValueError("Universal packaging requires macOS")
    targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"] if universal else [host]
    built_packages = set()
    metadata = None
    for target in targets:
        build = run(["cargo", "build", "--release", "--locked", "--features", "hid", "--target", target,
                     "--message-format=json", "-p", "openfuse-desktop", "-p", "openfuse-cli"],
                    cwd=ROOT, env=build_env, stdout=subprocess.PIPE)
        artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith("{")]
        built_packages.update(item["package_id"] for item in artifacts
                              if item.get("reason") == "compiler-artifact")
    # Unfiltered metadata includes dependencies of both Universal slices;
    # built_packages restricts notices to artifacts actually built above.
    metadata = json.loads(run(["cargo", "metadata", "--format-version", "1", "--locked",
                              "--features", "hid"], cwd=ROOT, capture_output=True).stdout)
    host = "universal-apple-darwin" if universal else host
    if source_hashes(ROOT) != before:
        raise ValueError("Sources changed during build; rerun packaging with a stable source tree")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    name = f"openfuse-{version}-{host}"
    output.mkdir(parents=True)
    staged = output / name
    staged.mkdir()
    if system == "Darwin":
        app = staged / "OpenFUSE.app"
        binaries = app / "Contents/MacOS"
        resources = app / "Contents/Resources"
        binaries.mkdir(parents=True)
        resources.mkdir(parents=True)
        (app / "Contents/Info.plist").write_bytes(plistlib.dumps(app_info(version, minimum_os)))
        mac_icon(resources / "openfuse.icns")
    else:
        binaries = staged / "bin"
        resources = staged
        binaries.mkdir()
    built = Path(metadata["target_directory"]) / host / "release"
    suffix = ".exe" if system == "Windows" else ""
    binary_hashes = {}
    smoke = {}
    for stem in ("openfuse-desktop", "openfuse"):
        source = built / (stem + suffix)
        target = binaries / source.name
        if universal:
            slices = [Path(metadata["target_directory"]) / architecture / "release" / stem
                      for architecture in targets]
            run(["lipo", "-create", *map(str, slices), "-output", str(target)])
            run(["lipo", str(target), "-verify_arch", "arm64", "x86_64"])
        else:
            shutil.copy2(source, target)
        binary_hashes[stem] = sha256(target)
        smoke[stem] = run([str(target), "--help"], capture_output=True).stdout
    shutil.copyfile(ROOT / "LICENSE", resources / "LICENSE")
    for notice_name in PROJECT_NOTICES:
        shutil.copyfile(ROOT / notice_name, resources / notice_name)
    shutil.copyfile(ROOT / "packaging/openfuse.svg", resources / "openfuse.svg")
    shutil.copyfile(ROOT / "packaging/README.md", resources / "INSTALL.md")
    if system == "Darwin":
        # Visible alongside the app in the mounted image, without Show Contents.
        shutil.copyfile(ROOT / "packaging/README.md", staged / "INSTALL.md")
        shutil.copyfile(ROOT / "DISCLAIMER.md", staged / "DISCLAIMER.md")
    copy_guides(ROOT, resources)
    notices = dependency_notices(metadata, resources / "third-party", built_packages)
    if notices["without_license_or_supplement"]:
        raise ValueError("Missing dependency license review: " + ", ".join(
            notices["without_license_or_supplement"]))
    if system == "Linux":
        for path in (ROOT / "packaging/linux").iterdir():
            if path.is_file():
                shutil.copy2(path, staged / path.name)
    if system == "Windows":
        shutil.copyfile(ROOT / "packaging/windows/register-file-type.ps1",
                        staged / "register-file-type.ps1")
    native = {}
    if system in ("Darwin", "Linux"):
        for path in binaries.iterdir():
            native[path.name] = run(["otool", "-L", str(path)] if system == "Darwin"
                                    else ["ldd", str(path)], capture_output=True).stdout
            if system == "Linux" and "not found" in native[path.name]:
                raise ValueError(f"Unresolved native library dependency for {path.name}")
    git = run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True).stdout.strip()
    provenance = {"schema": "openfuse.portable-build.v1", "version": version,
        "target": host, "architectures": targets, "system": system, "git_commit": git,
        "git_dirty": bool(run(["git", "status", "--porcelain"], cwd=ROOT,
                              capture_output=True).stdout.strip()),
        "source_sha256": before, "binary_sha256_before_signing": binary_hashes,
        "rustc": rustc, "features": ["hid"], "offline_help_smoke": smoke,
        "native_dependencies": native, "third_party": notices,
        "hardware_validation": "not performed", "gui_validation": "not performed by packager",
        "signing": "ad-hoc; not notarized" if system == "Darwin" else "unsigned"}
    write_json(resources / "build-manifest.json", provenance)
    write_checksums(resources)
    if system == "Darwin":
        for path in binaries.iterdir():
            run(["codesign", "--force", "--sign", "-", str(path)])
        run(["codesign", "--force", "--sign", "-", str(app)])
        run(["codesign", "--verify", "--deep", "--strict", str(app)])
        # Outer checksums cover signed code. Never mutate the app after signing.
        write_checksums(staged)
    extension = {"Darwin": ".dmg", "Linux": ".tar.gz", "Windows": ".zip"}[system]
    archive = output / (name + extension)
    make_archive(staged, archive, system)
    verification = (verify_mac_archive(archive) if system == "Darwin"
                    else {"staged_checksums_verified": verify_checksums(staged),
                          "gui_launch": "not tested here"})
    manifest = {"schema": "openfuse.portable-release.v1", "target": host,
                "archive": archive.name, "sha256": sha256(archive),
                "bytes": archive.stat().st_size,
                "signed_binary_sha256": {path.name: sha256(path) for path in binaries.iterdir()},
                "staged_directory": staged.name, "verification": verification,
                "build": provenance}
    write_json(output / "release-manifest.json", manifest)
    (output / "SHA256SUMS").write_text(f"{manifest['sha256']}  {archive.name}\n")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/distribution")
    parser.add_argument("--universal", action="store_true", help="Build both macOS architectures")
    args = parser.parse_args()
    try:
        result = package(args.output, universal=args.universal)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Packaging failed: {error}\n")
    print(json.dumps({key: result[key] for key in ("archive", "target", "sha256", "bytes")}, indent=2))


if __name__ == "__main__":
    main()
