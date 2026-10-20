import importlib.util
import json
from pathlib import Path
import plistlib
import tarfile
import tempfile
import unittest
from unittest import mock
import zipfile

ROOT = Path(__file__).resolve().parents[2]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


package = module("package_release", ROOT / "tools/package_release.py")
linux = module("register_desktop", ROOT / "packaging/linux/register-desktop.py")


class PackagingTests(unittest.TestCase):
    def test_mac_document_handler_is_alternate_and_xml(self):
        info = plistlib.loads(plistlib.dumps(package.app_info("0.1.0", "11.0")))
        self.assertEqual(info["CFBundleExecutable"], "openfuse-desktop")
        self.assertEqual(info["LSMinimumSystemVersion"], "11.0")
        self.assertEqual(info["CFBundleDocumentTypes"][0]["LSHandlerRank"], "Alternate")
        self.assertEqual(info["UTImportedTypeDeclarations"][0]["UTTypeConformsTo"], ["public.xml"])

    def test_checksums_detect_payload_change_and_do_not_include_themselves(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            payload = root / "preset.fuse"
            payload.write_text("original")
            self.assertEqual(package.write_checksums(root), 1)
            original = (root / "SHA256SUMS").read_text()
            self.assertIn(package.sha256(payload), original)
            self.assertEqual(package.verify_checksums(root), 1)
            payload.write_text("changed")
            self.assertNotIn(package.sha256(payload), original)
            with self.assertRaises(ValueError):
                package.verify_checksums(root)
            self.assertEqual(package.write_checksums(root), 1)

    def test_archives_keep_root_and_executable_permissions(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "openfuse-test"
            (root / "bin").mkdir(parents=True)
            binary = root / "bin/openfuse"
            binary.write_text("executable fixture")
            binary.chmod(0o755)
            for system, extension in (("Linux", ".tar.gz"), ("Windows", ".zip")):
                dest = Path(temp) / ("release" + extension)
                package.make_archive(root, dest, system)
                if system == "Linux":
                    with tarfile.open(dest) as archive:
                        entry = archive.getmember("openfuse-test/bin/openfuse")
                        self.assertEqual(entry.mode & 0o111, binary.stat().st_mode & 0o111)
                        self.assertEqual(archive.extractfile(entry).read(), binary.read_bytes())
                else:
                    with zipfile.ZipFile(dest) as archive:
                        self.assertEqual(archive.read("openfuse-test/bin/openfuse"), binary.read_bytes())

    def test_dependency_walk_skips_dev_and_unrelated_workspace_members(self):
        packages = [{"id": name, "name": name, "version": "1", "source": source}
                    for name, source in [("openfuse-desktop", None), ("runtime", "registry+x"),
                                         ("devonly", "registry+x"), ("unrelated", "registry+x")]]
        nodes = [{"id": p["id"], "deps": []} for p in packages]
        nodes[0]["deps"] = [
            {"pkg": "runtime", "dep_kinds": [{"kind": None}]},
            {"pkg": "devonly", "dep_kinds": [{"kind": "dev"}]}]
        metadata = {"packages": packages, "resolve": {"nodes": nodes}}
        self.assertEqual([p["name"] for p in package.dependency_packages(metadata)], ["runtime"])

    @unittest.skipIf(__import__("os").name == "nt", "POSIX path names and desktop registration")
    def test_linux_registration_is_opt_in_per_user_and_ownership_checked(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(linux.shutil, "which", return_value=None):
            root = Path(temp) / 'path with % " $ and spaces'
            (root / "bin").mkdir(parents=True)
            (root / "bin/openfuse-desktop").write_text("fixture")
            (root / "openfuse-mime.xml").write_text("mime fixture")
            data = Path(temp) / "user-data"
            linux.register(root, data, True)
            desktop = data / "applications/org.openfuse.editor.desktop"
            text = desktop.read_text()
            self.assertIn('%%', text)
            self.assertIn('\\"', text)
            self.assertIn('\\$', text)
            self.assertIn(' %f', text)
            self.assertFalse((data / "applications/mimeapps.list").exists())
            with self.assertRaises(ValueError):
                linux.register(Path(temp) / "other-install", data, False)
            self.assertTrue(desktop.exists())
            linux.register(root, data, False)
            self.assertFalse(desktop.exists())
            self.assertFalse((data / "mime/packages/org.openfuse.editor.xml").exists())

    @unittest.skipIf(__import__("os").name == "nt", "POSIX control characters in paths")
    def test_linux_rejects_newline_path_before_registration(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "bad\npath"
            (root / "bin").mkdir(parents=True)
            (root / "bin/openfuse-desktop").touch()
            with self.assertRaises(ValueError):
                linux.register(root, Path(temp) / "data", True)

    def test_notices_copy_exact_source_and_record_missing_license(self):
        with tempfile.TemporaryDirectory() as temp:
            registry = Path(temp) / "registry"
            source = registry / "src/test-registry/example-1.0"
            cache = registry / "cache/test-registry"
            source.mkdir(parents=True)
            cache.mkdir(parents=True)
            (source / "Cargo.toml").touch()
            archive = cache / "example-1.0.crate"
            archive.write_bytes(b"exact published archive fixture")
            dependency = {"id": "example", "name": "example", "version": "1.0",
                          "source": "registry+test", "manifest_path": str(source / "Cargo.toml"),
                          "license": "MIT", "authors": ["Example"], "repository": None}
            with mock.patch.object(package, "dependency_packages", return_value=[dependency]):
                destination = Path(temp) / "notices"
                report = package.dependency_notices({}, destination)
                self.assertEqual(report["without_standalone_license"], ["example-1.0"])
                inventory = json.loads((destination / "inventory.json").read_text())
                self.assertEqual(inventory[0]["source_sha256"], package.sha256(archive))
                self.assertEqual((destination / "sources/example-1.0.crate").read_bytes(), archive.read_bytes())

    def test_supplement_is_bound_to_crate_and_notice_bytes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "registry/src/test/example-1.0"
            source.mkdir(parents=True)
            archive = root / "registry/cache/test/example-1.0.crate"
            archive.parent.mkdir(parents=True)
            archive.write_bytes(b"exact archive")
            notices = root / "packaging/third-party"
            notices.mkdir(parents=True)
            notice = notices / "LICENSE.txt"
            notice.write_text("upstream notice")
            item = {"file": "LICENSE.txt", "sha256": package.sha256(notice)}
            entry = {"crate_sha256": package.sha256(archive), "files": [item]}
            manifest = notices / "supplemental.json"
            manifest.write_text(json.dumps({"packages": {"example-1.0": entry}}))
            dep = {"name": "example", "version": "1.0", "source": "registry+test",
                   "manifest_path": str(source / "Cargo.toml"), "license": "MIT",
                   "authors": ["Example"], "repository": None}
            with mock.patch.object(package, "ROOT", root), mock.patch.object(
                    package, "dependency_packages", return_value=[dep]):
                report = package.dependency_notices({}, root / "output")
                self.assertEqual(report["without_license_or_supplement"], [])
                self.assertEqual((root / "output/licenses/example-1.0/supplemental/LICENSE.txt").read_text(), "upstream notice")
                notice.write_text("changed notice")
                with self.assertRaisesRegex(ValueError, "hash mismatch"):
                    package.dependency_notices({}, root / "bad-notice")
                archive.write_bytes(b"different archive")
                with self.assertRaisesRegex(ValueError, "source mismatch"):
                    package.dependency_notices({}, root / "bad-crate")

    def test_supplement_rejects_absolute_paths_and_traversal(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "registry/src/test/example-1.0"
            source.mkdir(parents=True)
            archive = root / "registry/cache/test/example-1.0.crate"
            archive.parent.mkdir(parents=True)
            archive.write_bytes(b"archive")
            notices = root / "packaging/third-party"
            notices.mkdir(parents=True)
            dep = {"name": "example", "version": "1.0", "source": "registry+test",
                   "manifest_path": str(source / "Cargo.toml"), "license": "MIT",
                   "authors": [], "repository": None}
            for i, path in enumerate([str(notices / "LICENSE"), "../LICENSE"]):
                (notices / "supplemental.json").write_text(json.dumps({"packages": {
                    "example-1.0": {"crate_sha256": package.sha256(archive),
                                    "files": [{"file": path, "sha256": "unused"}]}}}))
                with mock.patch.object(package, "ROOT", root), mock.patch.object(
                        package, "dependency_packages", return_value=[dep]):
                    with self.assertRaisesRegex(ValueError, "path escape"):
                        package.dependency_notices({}, root / f"output-{i}")

    def test_unused_optional_dependencies_do_not_need_source_archives(self):
        with tempfile.TemporaryDirectory() as temp:
            unused = {"id": "optional-unused", "name": "unused", "version": "1.0",
                      "source": "registry+test", "manifest_path": "/absent/Cargo.toml"}
            with mock.patch.object(package, "dependency_packages", return_value=[unused]):
                report = package.dependency_notices({}, Path(temp) / "notices", {"actually-built"})
                self.assertEqual(report["packages"], 0)

    def test_checksum_verifier_rejects_path_escape(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "package"
            root.mkdir()
            outside = Path(temp) / "outside"
            outside.write_text("outside")
            (root / "SHA256SUMS").write_text(package.sha256(outside) + "  ../outside\n")
            with self.assertRaises(ValueError):
                package.verify_checksums(root)

    def test_bundled_guides_have_no_dead_local_markdown_links(self):
        with tempfile.TemporaryDirectory() as temp:
            destination = Path(temp)
            count = package.copy_guides(ROOT, destination)
            self.assertGreater(count, len(package.GUIDES))
            self.assertTrue((destination / "packaging/README.md").is_file())
            self.assertTrue((destination / "docs/usage.md").is_file())
            self.assertTrue((destination / "DISCLAIMER.md").is_file())
            for source in destination.rglob("*.md"):
                text = source.read_text(encoding="utf-8")
                for _, link in package.MARKDOWN_LINK.findall(text):
                    target = package.local_link(source, link)
                    if target is not None:
                        self.assertTrue(target.is_file(), (source, link))
                        self.assertTrue(target.is_relative_to(destination.resolve()), (source, link))
            self.assertFalse((destination / "research").exists())
            self.assertFalse((destination / "docs/reports").exists())
            self.assertFalse(package.allowed_guide(ROOT, ROOT / "docs/reports/report.md"))
            self.assertFalse(package.allowed_guide(ROOT, ROOT / "research/evidence/record.json"))
            self.assertFalse(list(destination.rglob("*.png")))


if __name__ == "__main__":
    unittest.main()
