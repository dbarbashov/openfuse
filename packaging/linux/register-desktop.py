#!/usr/bin/env python3
"""Explicit per-user application/MIME registration; never changes the default."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess

APP_ID = "org.openfuse.editor"


def quoted_exec(path):
    # Desktop Entry quoted-string rules; percent is a field-code escape.
    text = str(path).replace("%", "%%")
    for char in ("\\", '"', "`", "$"):
        text = text.replace(char, "\\" + char)
    # The desktop string layer unescapes before the Exec quoting layer.
    text = text.replace("\\", "\\\\")
    return '"' + text + '"'


def register(root, data, install):
    desktop = data / "applications" / (APP_ID + ".desktop")
    mime = data / "mime/packages" / (APP_ID + ".xml")
    marker = "X-OpenFUSE-Root=" + str(root)
    if desktop.exists() and marker not in desktop.read_text().splitlines():
        raise ValueError("Another installation owns the desktop entry; unregister it first")
    if install:
        executable = root / "bin/openfuse-desktop"
        if not executable.is_file():
            raise ValueError("Packaged executable is missing")
        if any(char in str(root) for char in "\n\r\t="):
            raise ValueError("Installation path contains a control character or '='")
        desktop.parent.mkdir(parents=True, exist_ok=True)
        mime.parent.mkdir(parents=True, exist_ok=True)
        desktop.write_text("\n".join([
            "[Desktop Entry]", "Type=Application", "Name=OpenFUSE",
            "Comment=Legacy amplifier preset editor", "Terminal=false",
            "Icon=" + str(root / "openfuse.svg").replace("\\", "\\\\"),
            "Exec=" + quoted_exec(executable) + " %f",
            "MimeType=application/x-openfuse-preset;", "Categories=AudioVideo;Audio;",
            marker, "",
        ]))
        shutil.copyfile(root / "openfuse-mime.xml", mime)
    else:
        if desktop.exists():
            desktop.unlink()
            mime.unlink(missing_ok=True)
    for command, folder in (("update-desktop-database", data / "applications"),
                            ("update-mime-database", data / "mime")):
        if shutil.which(command) and folder.is_dir():
            subprocess.run([command, str(folder)], check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["install", "uninstall"])
    args = parser.parse_args()
    base = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share"))
    if not base.is_absolute():
        parser.error("XDG_DATA_HOME must be absolute")
    register(Path(__file__).resolve().parent, base, args.action == "install")
