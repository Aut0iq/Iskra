#!/usr/bin/env python3
"""Packaging tests for the Iskra plugin + desktop companion.

Runs entirely in temp directories: never touches ~/.zeron, ~/.local,
cargo, or the network. Uses unittest only.

    python3 scripts/tests/test_packaging.py
"""

import hashlib
import json
import os
import shutil
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zlib
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SCRIPTS))
import lib_assets
import lib_licenses

REPO = SCRIPTS.parent


def make_png(w=256, h=256, alpha=255):
    """Minimal valid RGBA8 non-interlaced PNG (filter 0 rows)."""
    def chunk(ctype, body):
        return (
            struct.pack(">I", len(body))
            + ctype
            + body
            + struct.pack(">I", zlib.crc32(ctype + body) & 0xFFFFFFFF)
        )
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)
    row = b"\x00" + bytes([16, 32, 48, alpha]) * w
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(row * h))
        + chunk(b"IEND", b"")
    )


ALL_NAMES = sorted(lib_assets.REQUIRED_ANIMATIONS)


def manifest_dict(names=None, frames_per=8):
    names = names if names is not None else ALL_NAMES
    anims = {}
    for n in names:
        anims[n] = {
            "frames": [
                f"build-test/frames/{n}/{i:02d}.png" for i in range(frames_per)
            ],
            "frame_ms": 120,
            "playback": "loop",
            "contact": [128, 248],
        }
    return {
        "version": 2,
        "profile": "iskra-v6",
        "frame_width": 256,
        "frame_height": 256,
        "logical_width": 160,
        "logical_height": 160,
        "animations": anims,
    }


def write_manifest(v6, manifest):
    v6.mkdir(parents=True, exist_ok=True)
    (v6 / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")


def write_frames(v6, manifest, png=None):
    png = png if png is not None else make_png()
    for entry in manifest["animations"].values():
        for rel in entry["frames"]:
            p = v6 / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(png)


def write_license_snapshot(root, lock_content=b"[fixture lock]\n"):
    """A tiny synthetic Cargo.lock plus a one-package license snapshot
    pinned to it — the same schema vendor-license-texts.py emits."""
    root = Path(root)
    lock = root / "Cargo.lock"
    lock.write_bytes(lock_content)
    snap = root / "third_party/rust-licenses"
    pkg = snap / "demo-crate-1.0.0"
    pkg.mkdir(parents=True)
    lic = pkg / "LICENSE"
    lic.write_text("fixture license text\n", encoding="utf-8")
    manifest = {
        "schema": 1,
        "target": "x86_64-unknown-linux-gnu",
        "scope": "fixture",
        "cargo_lock_sha256": hashlib.sha256(
            lock.read_bytes()
        ).hexdigest(),
        "packages": [
            {
                "name": "demo-crate",
                "version": "1.0.0",
                "license": "MIT",
                "files": [
                    {
                        "path": "demo-crate-1.0.0/LICENSE",
                        "sha256": hashlib.sha256(
                            lic.read_bytes()
                        ).hexdigest(),
                    }
                ],
            }
        ],
    }
    (snap / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return snap, lock


ROOT_LICENSE_FILES = ("LICENSE", "LICENSE-ARTWORK", "ARTWORK.md", "TRADEMARKS.md")

LICENSE_FIXTURES = {
    "LICENSE": "MIT License\n\nfixture code/docs license text\n",
    "LICENSE-ARTWORK": "CC BY 4.0\n\nfixture artwork license text\n",
    "ARTWORK.md": "# Artwork\n\nfixture artwork license notes\n",
    "TRADEMARKS.md": "# Trademarks\n\nfixture third-party marks notice\n",
}

PLUGIN_NOTICES = "# Third-party notices\n\nfixture SDK MIT notice\n"


def write_license_files(root):
    """Tiny stand-ins for the repo-root release license files."""
    for name, text in LICENSE_FIXTURES.items():
        (Path(root) / name).write_text(text, encoding="utf-8")


def fixture_repo(root, with_plugin=True, with_binary_assets=True):
    """assets/v6 with real tiny PNGs + Cargo.toml (+ plugin sources)."""
    root = Path(root)
    v6 = root / "assets/v6"
    m = manifest_dict()
    write_manifest(v6, m)
    if with_binary_assets:
        write_frames(v6, m)
    (root / "Cargo.toml").write_text(
        '[package]\nname = "zeron-pet"\nversion = "9.9.9"\n', encoding="utf-8"
    )
    if with_plugin:
        p = root / "plugin/iskra"
        (p / "locales").mkdir(parents=True)
        (p / "zeron-plugin.json").write_text(
            json.dumps(
                {
                    "id": "iskra",
                    "name": "Iskra",
                    "version": "9.9.9",
                    "entry": "init.luau",
                    "permissions": ["ui.add", "commands"],
                    "commands": ["zeron-pet"],
                    "signature": {"key": "stale", "files": {}, "sig": "stale"},
                }
            ),
            encoding="utf-8",
        )
        (p / "init.luau").write_text("return {}\n", encoding="utf-8")
        (p / "zeron.luau").write_text("-- sdk\n", encoding="utf-8")
        (p / "model.luau").write_text("-- model\n", encoding="utf-8")
        (p / "THIRD_PARTY_NOTICES.md").write_text(
            PLUGIN_NOTICES, encoding="utf-8"
        )
        (p / "locales/en.json").write_text("{}\n", encoding="utf-8")
        # Things that must never reach the staged folder:
        (p / ".hidden").write_text("x")
        (p / "tests").mkdir()
        (p / "tests/test_x.py").write_text("x")
        (p / "token.key").write_text("x")
        (p / "client_secret.txt").write_text("x")
    write_license_files(root)
    write_license_snapshot(root)
    return m


class ManifestValidationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="iskra-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.v6 = self.tmp / "assets/v6"

    def _manifest_with(self, anim, index, value):
        m = manifest_dict()
        m["animations"][anim]["frames"][index] = value
        write_manifest(self.v6, m)
        return self.v6 / "manifest.json"

    def test_traversal_rejected(self):
        path = self._manifest_with("dragged", 0, "../escape.png")
        with self.assertRaises(lib_assets.AssetError):
            lib_assets.load_manifest(path)

    def test_deep_traversal_rejected(self):
        path = self._manifest_with("dragged", 0, "frames/../../escape.png")
        with self.assertRaises(lib_assets.AssetError):
            lib_assets.load_manifest(path)

    def test_absolute_rejected(self):
        path = self._manifest_with("dragged", 0, "/etc/passwd")
        with self.assertRaises(lib_assets.AssetError):
            lib_assets.load_manifest(path)

    def test_quoted_frame_path_rejected(self):
        for bad in ('frames/x"q.png', "frames/x'y.png", "frames/x`y.png",
                    "frames/x\ny.png", "frames/x y.png", "frames\\x.png"):
            with self.subTest(bad=bad):
                path = self._manifest_with("dragged", 0, bad)
                with self.assertRaises(lib_assets.AssetError):
                    lib_assets.load_manifest(path)

    def test_wrong_frame_count_rejected(self):
        m = manifest_dict(frames_per=7)
        write_manifest(self.v6, m)
        with self.assertRaises(lib_assets.AssetError):
            lib_assets.load_manifest(self.v6 / "manifest.json")

    def test_missing_required_animation_rejected(self):
        names = [n for n in ALL_NAMES if n != "floor_recline"]
        m = manifest_dict(names=names)
        write_manifest(self.v6, m)
        with self.assertRaises(lib_assets.AssetError) as ctx:
            lib_assets.load_manifest(self.v6 / "manifest.json")
        self.assertIn("floor_recline", str(ctx.exception))

    def test_extra_animation_key_rejected(self):
        m = manifest_dict()
        m["animations"]["evil_inject"] = {
            "frames": ["build-test/frames/dragged/00.png"] * 8,
            "frame_ms": 100,
        }
        write_manifest(self.v6, m)
        with self.assertRaises(lib_assets.AssetError) as ctx:
            lib_assets.load_manifest(self.v6 / "manifest.json")
        self.assertIn("evil_inject", str(ctx.exception))

    def test_missing_frame_rejected(self):
        m = manifest_dict()
        write_manifest(self.v6, m)
        write_frames(self.v6, m)
        # delete one referenced frame
        target = self.v6 / m["animations"]["dragged"]["frames"][3]
        target.unlink()
        with self.assertRaises(lib_assets.AssetError):
            lib_assets.validate_frames(
                lib_assets.load_manifest(self.v6 / "manifest.json"), self.v6
            )

    def test_symlink_escape_rejected(self):
        m = manifest_dict()
        write_manifest(self.v6, m)
        write_frames(self.v6, m)
        outside = self.tmp / "outside.png"
        outside.write_bytes(make_png())
        victim = self.v6 / m["animations"]["dragged"]["frames"][0]
        victim.unlink()
        victim.symlink_to(outside)
        with self.assertRaises(lib_assets.AssetError):
            lib_assets.validate_frames(
                lib_assets.load_manifest(self.v6 / "manifest.json"), self.v6
            )

    def test_bad_png_dimensions_rejected(self):
        m = manifest_dict()
        write_manifest(self.v6, m)
        write_frames(self.v6, m, png=make_png(128, 128))
        with self.assertRaises(lib_assets.AssetError):
            lib_assets.validate_frames(
                lib_assets.load_manifest(self.v6 / "manifest.json"), self.v6
            )

    def test_transparent_png_rejected(self):
        m = manifest_dict()
        write_manifest(self.v6, m)
        write_frames(self.v6, m, png=make_png(alpha=0))
        with self.assertRaises(lib_assets.AssetError):
            lib_assets.validate_frames(
                lib_assets.load_manifest(self.v6 / "manifest.json"), self.v6
            )

    def test_unsafe_version_rejected(self):
        for bad in ("../x", "9.9.9/../x", "9.9.9\nx", "9.9.9 x",
                    "9.9.9/../../etc", "abc", "1.2", ""):
            with self.subTest(bad=bad):
                with self.assertRaises(lib_assets.AssetError):
                    lib_assets.check_version(bad)
        for good in ("0.1.0", "9.9.9", "1.0.0-rc.1", "2.0.0-beta_2"):
            self.assertEqual(lib_assets.check_version(good), good)


class PluginBuildTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="iskra-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def _run(self, repo, *extra):
        return subprocess.run(
            [sys.executable, str(SCRIPTS / "build-plugin.py"), "--repo-root",
             str(repo), "--wait", "0", *extra],
            capture_output=True, text=True,
        )

    def test_full_build_stages_unsigned_and_generates_assets(self):
        repo = self.tmp / "repo"
        fixture_repo(repo)
        src_manifest = repo / "assets/v6/manifest.json"
        before = src_manifest.read_bytes()
        r = self._run(repo)
        self.assertEqual(r.returncode, 0, r.stderr)
        # Source untouched.
        self.assertEqual(src_manifest.read_bytes(), before)

        assets = repo / "plugin/iskra/assets"
        mod = (assets / "animations.luau").read_text()
        self.assertIn("return {", mod)
        self.assertIn('frame_ms = 120', mod)
        self.assertIn('"assets/v6/build-test/frames/dragged/00.png"', mod)
        for n in ALL_NAMES:
            self.assertRegex(mod, rf"(?m)^    {n} = {{$")
        self.assertEqual(
            len(list((assets / "v6").rglob("*.png"))),
            38 * 8,
        )
        self.assertTrue((assets / "manifest.json").is_file())

        stage = repo / "dist/iskra-9.9.9"
        self.assertTrue(stage.is_dir())
        staged_manifest = json.loads(
            (stage / "zeron-plugin.json").read_text()
        )
        self.assertNotIn("signature", staged_manifest)
        self.assertEqual(staged_manifest["commands"], ["zeron-pet"])
        self.assertTrue((stage / "init.luau").is_file())
        self.assertTrue((stage / "zeron.luau").is_file())
        self.assertTrue((stage / "model.luau").is_file())
        self.assertTrue((stage / "locales/en.json").is_file())
        self.assertTrue(
            (stage / "assets/v6/build-test/frames/dragged/00.png").is_file()
        )
        # Repo-root release files ship byte-exact in the staged plugin;
        # the SDK's own third-party notice file rides along too.
        for name in ROOT_LICENSE_FILES:
            self.assertEqual(
                (stage / name).read_bytes(),
                (repo / name).read_bytes(),
                name,
            )
        self.assertEqual(
            (stage / "THIRD_PARTY_NOTICES.md").read_bytes(),
            (repo / "plugin/iskra/THIRD_PARTY_NOTICES.md").read_bytes(),
        )
        # Artifact root is exactly the expected set — no stray marker or
        # placeholder file is ever added.
        self.assertEqual(
            sorted(p.name for p in stage.iterdir()),
            sorted(
                ["zeron-plugin.json", "init.luau", "zeron.luau",
                 "model.luau", "THIRD_PARTY_NOTICES.md", "locales",
                 "assets"]
                + list(ROOT_LICENSE_FILES)
            ),
        )
        # Exclusions: tests, dotfiles, secrets, keys.
        self.assertFalse((stage / "tests").exists())
        self.assertFalse((stage / ".hidden").exists())
        self.assertFalse((stage / "token.key").exists())
        self.assertFalse((stage / "client_secret.txt").exists())
        self.assertIn("UNSIGNED staging ready", r.stdout)

    def test_rebuild_deterministic_and_old_assets_preserved(self):
        repo = self.tmp / "repo"
        fixture_repo(repo)
        r1 = self._run(repo)
        self.assertEqual(r1.returncode, 0, r1.stderr)
        mod1 = (repo / "plugin/iskra/assets/animations.luau").read_bytes()
        r2 = self._run(repo)
        self.assertEqual(r2.returncode, 0, r2.stderr)
        mod2 = (repo / "plugin/iskra/assets/animations.luau").read_bytes()
        self.assertEqual(mod1, mod2)
        # Previous generated dir preserved to a dot-backup, never deleted.
        backups = list(
            (repo / "plugin/iskra").glob(".assets.bak-*")
        )
        self.assertEqual(len(backups), 1)
        self.assertTrue((backups[0] / "GENERATED").is_file())
        # The backup is dot-excluded from the staged plugin folder.
        stage = repo / "dist/iskra-9.9.9"
        self.assertFalse(list(stage.glob("*.bak-*")))
        self.assertFalse(list(stage.glob(".assets.bak-*")))

    def test_existing_unowned_stage_dir_preserved_not_deleted(self):
        repo = self.tmp / "repo"
        fixture_repo(repo)
        out = repo / "out"
        clash = out / "iskra-9.9.9"
        clash.mkdir(parents=True)
        (clash / "user-file.txt").write_text("keep me")
        r = self._run(repo, "--output", str(out))
        self.assertEqual(r.returncode, 0, r.stderr)
        # New stage dir exists AND the old folder survived as a backup.
        self.assertTrue((clash / "zeron-plugin.json").is_file())
        backups = [d for d in out.glob("iskra-9.9.9.bak-*")]
        self.assertEqual(len(backups), 1)
        self.assertEqual(
            (backups[0] / "user-file.txt").read_text(), "keep me"
        )

    def test_plugin_symlink_aborts_stage(self):
        repo = self.tmp / "repo"
        fixture_repo(repo)
        outside = self.tmp / "secret-data"
        outside.write_text("sensitive")
        (repo / "plugin/iskra/leak.luau").symlink_to(outside)
        r = self._run(repo)
        self.assertEqual(r.returncode, 2)
        self.assertIn("symlink", (r.stderr + r.stdout).lower())
        self.assertFalse((repo / "dist/iskra-9.9.9").exists())

    def test_manifest_id_and_version_injection_rejected(self):
        for patch in ({"id": "other"}, {"version": "../escape"},
                      {"version": "9.9.9/../x"}, {"version": "not-semver"}):
            with self.subTest(patch=patch):
                repo = self.tmp / f"repo-{patch}"
                fixture_repo(repo)
                mp = repo / "plugin/iskra/zeron-plugin.json"
                m = json.loads(mp.read_text())
                m.update(patch)
                mp.write_text(json.dumps(m))
                r = self._run(repo)
                self.assertNotEqual(r.returncode, 0)
                self.assertFalse((repo / "dist").exists())

    def test_unexpected_files_skipped_with_warning(self):
        repo = self.tmp / "repo"
        fixture_repo(repo)
        (repo / "plugin/iskra/random.bin").write_bytes(b"\x00\x01")
        r = self._run(repo)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("skipped unexpected file: random.bin", r.stdout)
        self.assertFalse((repo / "dist/iskra-9.9.9/random.bin").exists())

    def test_missing_root_license_file_fails_closed(self):
        for name in ROOT_LICENSE_FILES:
            with self.subTest(name=name):
                repo = self.tmp / f"repo-no-{name}"
                fixture_repo(repo)
                (repo / name).unlink()
                r = self._run(repo)
                self.assertEqual(r.returncode, 2)
                self.assertIn(name, r.stderr)
                self.assertFalse((repo / "dist/iskra-9.9.9").exists())

    def test_symlinked_root_license_file_fails_closed(self):
        repo = self.tmp / "repo"
        fixture_repo(repo)
        victim = repo / "LICENSE"
        victim.unlink()
        victim.symlink_to(self.tmp / "elsewhere")
        r = self._run(repo)
        self.assertEqual(r.returncode, 2)
        self.assertIn("symlink", r.stderr.lower())
        self.assertFalse((repo / "dist/iskra-9.9.9").exists())

    def test_missing_plugin_sources_reports_but_keeps_assets(self):
        repo = self.tmp / "repo"
        fixture_repo(repo, with_plugin=False)
        r = self._run(repo)
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertIn("plugin sources not present", r.stderr)
        self.assertTrue(
            (repo / "plugin/iskra/assets/animations.luau").is_file()
        )


class CompanionBuildTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = Path(tempfile.mkdtemp(prefix="iskra-test-"))
        cls.repo = cls.tmp / "repo"
        cls.out = cls.tmp / "dist"
        fixture_repo(cls.repo, with_plugin=False)
        cls.fakebin = cls.tmp / "zeron-pet"
        cls.fakebin.write_text("#!/bin/sh\necho fixture\n")
        cls.fakebin.chmod(0o755)
        cls.env = dict(os.environ, SOURCE_DATE_EPOCH="1700000000")

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, True)

    def _build(self, out):
        return subprocess.run(
            ["sh", str(SCRIPTS / "build-companion.sh"),
             "--repo-root", str(self.repo),
             "--output", str(out),
             "--binary", str(self.fakebin)],
            capture_output=True, text=True, env=self.env,
        )

    def test_tarball_layout_members_and_modes(self):
        r = self._build(self.out)
        self.assertEqual(r.returncode, 0, r.stderr)
        archive = self.out / "iskra-desktop-9.9.9-linux-x86_64.tar.gz"
        self.assertTrue(archive.is_file())
        with tarfile.open(archive) as tf:
            members = tf.getmembers()
            for m in members:
                self.assertFalse(m.name.startswith("/"))
                self.assertNotIn("..", m.name.split("/"))
                self.assertFalse(m.issym() or m.islnk())
            files = {m.name for m in members if m.isfile()}
            root = "iskra-desktop-9.9.9"
            expected = {
                f"{root}/bin/zeron-pet",
                f"{root}/assets/v6/manifest.json",
                f"{root}/install.sh",
                f"{root}/README.md",
                f"{root}/THIRD_PARTY_NOTICES.md",
                f"{root}/FIXTURE-BUILD.txt",
                f"{root}/licenses/manifest.json",
                f"{root}/licenses/demo-crate-1.0.0/LICENSE",
            } | {
                f"{root}/{name}" for name in ROOT_LICENSE_FILES
            } | {
                f"{root}/assets/v6/build-test/frames/{n}/{i:02d}.png"
                for n in ALL_NAMES for i in range(8)
            }
            self.assertEqual(files, expected)
            # Release license files ship byte-exact at the package root.
            for name in ROOT_LICENSE_FILES:
                self.assertEqual(
                    tf.extractfile(f"{root}/{name}").read(),
                    (self.repo / name).read_bytes(),
                    name,
                )
            mode = tf.getmember(f"{root}/bin/zeron-pet").mode
            self.assertEqual(mode & 0o111, 0o111)
            imode = tf.getmember(f"{root}/install.sh").mode
            self.assertEqual(imode & 0o111, 0o111)

    def test_deterministic_archive(self):
        out2 = self.tmp / "dist2"
        r1 = self._build(self.out / "d1")
        r2 = self._build(out2)
        self.assertEqual(r1.returncode, 0, r1.stderr)
        self.assertEqual(r2.returncode, 0, r2.stderr)
        a = (self.out / "d1/iskra-desktop-9.9.9-linux-x86_64.tar.gz").read_bytes()
        b = (out2 / "iskra-desktop-9.9.9-linux-x86_64.tar.gz").read_bytes()
        self.assertEqual(a, b)

    def test_missing_root_license_file_fails_build(self):
        repo = self.tmp / "repo-norootlic"
        fixture_repo(repo, with_plugin=False)
        (repo / "TRADEMARKS.md").unlink()
        out = self.tmp / "dist-norootlic"
        r = subprocess.run(
            ["sh", str(SCRIPTS / "build-companion.sh"),
             "--repo-root", str(repo),
             "--output", str(out),
             "--binary", str(self.fakebin)],
            capture_output=True, text=True, env=self.env,
        )
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("TRADEMARKS.md", r.stderr + r.stdout)
        self.assertFalse(
            (out / "iskra-desktop-9.9.9-linux-x86_64.tar.gz").exists()
        )

    def test_symlinked_root_license_file_fails_build(self):
        repo = self.tmp / "repo-liclink"
        fixture_repo(repo, with_plugin=False)
        victim = repo / "LICENSE-ARTWORK"
        victim.unlink()
        victim.symlink_to(self.tmp / "elsewhere")
        out = self.tmp / "dist-liclink"
        r = subprocess.run(
            ["sh", str(SCRIPTS / "build-companion.sh"),
             "--repo-root", str(repo),
             "--output", str(out),
             "--binary", str(self.fakebin)],
            capture_output=True, text=True, env=self.env,
        )
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("symlink", (r.stderr + r.stdout).lower())
        self.assertFalse(
            (out / "iskra-desktop-9.9.9-linux-x86_64.tar.gz").exists()
        )

    def test_missing_license_snapshot_fails_build(self):
        # Even in --binary fixture mode a missing snapshot must stop the
        # build, loudly, before any archive appears.
        repo = self.tmp / "repo-nolic"
        fixture_repo(repo, with_plugin=False)
        shutil.rmtree(repo / "third_party")
        out = self.tmp / "dist-nolic"
        r = subprocess.run(
            ["sh", str(SCRIPTS / "build-companion.sh"),
             "--repo-root", str(repo),
             "--output", str(out),
             "--binary", str(self.fakebin)],
            capture_output=True, text=True, env=self.env,
        )
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("license", (r.stderr + r.stdout).lower())
        self.assertFalse(
            (out / "iskra-desktop-9.9.9-linux-x86_64.tar.gz").exists()
        )

    def test_stale_lockfile_fails_build(self):
        repo = self.tmp / "repo-stale"
        fixture_repo(repo, with_plugin=False)
        # A lockfile different from the pinned snapshot = stale snapshot.
        (repo / "Cargo.lock").write_bytes(b"[changed lock]\n")
        out = self.tmp / "dist-stale"
        r = subprocess.run(
            ["sh", str(SCRIPTS / "build-companion.sh"),
             "--repo-root", str(repo),
             "--output", str(out),
             "--binary", str(self.fakebin)],
            capture_output=True, text=True, env=self.env,
        )
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("stale", (r.stderr + r.stdout).lower())

    def _fail_tool_build(self, tool, body, tag):
        """Put a failing fake `tool` first on PATH against a repo with a
        preexisting final sentinel archive: the build must fail and the
        sentinel must survive byte-exact with no stray temp files."""
        repo = self.tmp / f"repo-{tag}"
        fixture_repo(repo, with_plugin=False)
        out = self.tmp / f"dist-{tag}"
        out.mkdir()
        archive = out / "iskra-desktop-9.9.9-linux-x86_64.tar.gz"
        sentinel = b"previous final archive\n"
        archive.write_bytes(sentinel)
        fakebin = self.tmp / f"fakebin-{tag}"
        fakebin.mkdir()
        fake = fakebin / tool
        fake.write_text(body)
        fake.chmod(0o755)
        env = dict(self.env, PATH=f"{fakebin}:{self.env['PATH']}")
        r = subprocess.run(
            ["sh", str(SCRIPTS / "build-companion.sh"),
             "--repo-root", str(repo),
             "--output", str(out),
             "--binary", str(self.fakebin)],
            capture_output=True, text=True, env=env,
        )
        self.assertNotEqual(r.returncode, 0)
        # The preexisting final archive is preserved byte-exact and the
        # private stage dir left nothing behind in the output dir.
        self.assertEqual(archive.read_bytes(), sentinel)
        self.assertEqual(
            sorted(p.name for p in out.iterdir()), [archive.name]
        )

    def test_tar_failure_preserves_previous_archive(self):
        # Fake tar writes a syntactically valid (empty) tar to the -cf
        # target but exits 42 — the sequential tar step must be fatal
        # even though a following gzip would have succeeded.
        fake_tar = (
            "#!/bin/sh\n"
            "out=\n"
            "while [ $# -gt 0 ]; do\n"
            "  if [ \"$1\" = -cf ]; then shift; out=$1; fi\n"
            "  shift\n"
            "done\n"
            "[ -n \"$out\" ] && head -c 10240 /dev/zero > \"$out\"\n"
            "exit 42\n"
        )
        self._fail_tool_build("tar", fake_tar, "tarfail")

    def test_gzip_failure_preserves_previous_archive(self):
        fake_gzip = "#!/bin/sh\ncat >/dev/null 2>&1 || true\nexit 42\n"
        self._fail_tool_build("gzip", fake_gzip, "gzipfail")


class InstallerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = Path(tempfile.mkdtemp(prefix="iskra-test-"))
        cls.repo = cls.tmp / "repo"
        fixture_repo(cls.repo, with_plugin=False)
        fakebin = cls.tmp / "zeron-pet"
        fakebin.write_text("#!/bin/sh\necho fixture\n")
        fakebin.chmod(0o755)
        dist = cls.tmp / "dist"
        r = subprocess.run(
            ["sh", str(SCRIPTS / "build-companion.sh"),
             "--repo-root", str(cls.repo),
             "--output", str(dist),
             "--binary", str(fakebin)],
            capture_output=True, text=True,
        )
        if r.returncode != 0:
            raise RuntimeError(r.stderr)
        with tarfile.open(dist / "iskra-desktop-9.9.9-linux-x86_64.tar.gz") as tf:
            tf.extractall(cls.tmp / "pkg")
        cls.pkg = cls.tmp / "pkg/iskra-desktop-9.9.9"

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, True)

    def setUp(self):
        self.pfx = self.tmp / f"pfx-{self._testMethodName}"
        self.pfx.mkdir(parents=True, exist_ok=True)
        self.env = dict(os.environ, HOME=str(self.tmp))
        self.dest = self.pfx / "lib/iskra-desktop/9.9.9"
        self.link = self.pfx / "bin/zeron-pet"

    def _install(self, *extra, env=None):
        return subprocess.run(
            ["sh", str(self.pkg / "install.sh"), "--prefix", str(self.pfx),
             *extra],
            capture_output=True, text=True, env=env or self.env,
        )

    def test_install_temp_prefix(self):
        r = self._install()
        self.assertEqual(r.returncode, 0, r.stderr)
        exe = self.dest / "bin/zeron-pet"
        self.assertTrue(exe.is_file())
        self.assertTrue(os.access(exe, os.X_OK))
        self.assertTrue((self.dest / "assets/v6/manifest.json").is_file())
        self.assertTrue((self.dest / ".iskra-desktop-install").is_file())
        self.assertTrue(self.link.is_symlink())
        self.assertEqual(
            os.readlink(self.link), str(self.dest / "bin/zeron-pet")
        )
        # Nothing outside prefix structure, no autostart artifacts.
        self.assertFalse((self.pfx / ".config").exists())
        self.assertFalse((self.tmp / ".config/systemd").exists())
        self.assertFalse((self.tmp / ".config/autostart").exists())

    def test_installed_notices_and_dependency_snapshot_are_retained(self):
        result = self._install()
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in ("LICENSE", "LICENSE-ARTWORK", "ARTWORK.md", "TRADEMARKS.md"):
            self.assertEqual((self.dest / name).read_bytes(), (self.pkg / name).read_bytes())
        for source in (self.pkg / "licenses").rglob("*"):
            if source.is_file():
                self.assertEqual((self.dest / "licenses" / source.relative_to(self.pkg / "licenses")).read_bytes(), source.read_bytes())

    def test_install_idempotent_no_backup(self):
        r1 = self._install()
        self.assertEqual(r1.returncode, 0, r1.stderr)
        r2 = self._install()
        self.assertEqual(r2.returncode, 0, r2.stderr)
        self.assertIn("unchanged", r2.stdout + r2.stderr)
        # Identical reinstall → no backup was needed or made.
        self.assertFalse(
            (self.pfx / "lib/iskra-desktop/.backups").exists()
        )

    def test_changed_reinstall_preserves_old_to_backup(self):
        r1 = self._install()
        self.assertEqual(r1.returncode, 0, r1.stderr)
        # Change the package source so content differs.
        readme = self.pkg / "README.md"
        orig = readme.read_bytes()
        readme.write_text("changed readme")
        try:
            r2 = self._install()
            self.assertEqual(r2.returncode, 0, r2.stderr)
            bks = list(
                (self.pfx / "lib/iskra-desktop/.backups").glob("9.9.9-*")
            )
            self.assertEqual(len(bks), 1)
            self.assertTrue(
                (bks[0] / "9.9.9/.iskra-desktop-install").is_file()
            )
            self.assertEqual(
                (self.dest / "README.md").read_text(), "changed readme"
            )
        finally:
            readme.write_bytes(orig)

    def test_foreign_link_preserved_and_refused(self):
        (self.pfx / "bin").mkdir(parents=True)
        self.link.write_text("user file")
        r = self._install()
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(self.link.read_text(), "user file")
        self.assertFalse(self.dest.exists())

    def test_managed_dest_plus_foreign_link_refused_dest_unchanged(self):
        r1 = self._install()
        self.assertEqual(r1.returncode, 0, r1.stderr)
        sentinel = self.dest / "bin/zeron-pet"
        before = sentinel.read_bytes()
        # Replace the managed link with a foreign regular file.
        self.link.unlink()
        self.link.write_text("not ours")
        r2 = self._install()
        self.assertNotEqual(r2.returncode, 0)
        self.assertEqual(self.link.read_text(), "not ours")
        self.assertEqual(sentinel.read_bytes(), before)
        # No backup was made either — refusal happened pre-mutation.
        self.assertFalse(
            (self.pfx / "lib/iskra-desktop/.backups").exists()
        )

    def test_dest_symlink_refused(self):
        real = self.tmp / "elsewhere"
        real.mkdir()
        self.dest.parent.mkdir(parents=True)
        self.dest.symlink_to(real)
        r = self._install()
        self.assertNotEqual(r.returncode, 0)
        self.assertTrue(self.dest.is_symlink())
        self.assertFalse((real / "bin").exists())

    def test_unowned_destination_refused(self):
        self.dest.mkdir(parents=True)
        (self.dest / "mine.txt").write_text("keep me")
        r = self._install()
        self.assertNotEqual(r.returncode, 0)
        self.assertTrue((self.dest / "mine.txt").is_file())

    def test_dest_marker_mismatch_refused(self):
        self.dest.mkdir(parents=True)
        (self.dest / ".iskra-desktop-install").write_text("0.0.0")
        r = self._install()
        self.assertNotEqual(r.returncode, 0)

    def test_version_traversal_rejected(self):
        for bad in ("../evil", "9.9.9/../x", "9.9.9\nx", "1.2",
                    "9.9.9/../../etc"):
            with self.subTest(bad=bad):
                env = dict(self.env, ISKRA_DESKTOP_VERSION=bad)
                r = self._install(env=env)
                self.assertNotEqual(r.returncode, 0)
                self.assertFalse(
                    (self.pfx / "lib/iskra-desktop").exists()
                )
                self.assertFalse(self.link.exists())

    def test_relative_prefix_normalized(self):
        rel = self.tmp / "rel-home"
        rel.mkdir()
        r = subprocess.run(
            ["sh", str(self.pkg / "install.sh"), "--prefix", "relpfx"],
            capture_output=True, text=True, cwd=rel, env=self.env,
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        link = rel / "relpfx/bin/zeron-pet"
        self.assertTrue(link.is_symlink())
        target = os.readlink(link)
        self.assertTrue(os.path.isabs(target))
        self.assertTrue(os.path.exists(target))

    def test_rerun_installed_install_sh_noop(self):
        r1 = self._install()
        self.assertEqual(r1.returncode, 0, r1.stderr)
        marker = self.dest / ".iskra-desktop-install"
        before = marker.read_bytes()
        r2 = subprocess.run(
            ["sh", str(self.dest / "install.sh"), "--prefix", str(self.pfx)],
            capture_output=True, text=True, env=self.env,
        )
        self.assertEqual(r2.returncode, 0, r2.stderr)
        # Functional safe outcome: identical tree stays put, link intact
        # and correct, no backup churned, source not destroyed.
        self.assertIn("unchanged", r2.stdout + r2.stderr)
        self.assertTrue(marker.is_file())
        self.assertEqual(marker.read_bytes(), before)
        self.assertTrue(self.link.is_symlink())
        self.assertEqual(
            os.readlink(self.link), str(self.dest / "bin/zeron-pet")
        )
        self.assertTrue((self.dest / "bin/zeron-pet").is_file())
        self.assertFalse(
            (self.pfx / "lib/iskra-desktop/.backups").exists()
        )

    def test_reinstall_recreates_missing_link(self):
        r1 = self._install()
        self.assertEqual(r1.returncode, 0, r1.stderr)
        self.link.unlink()
        r2 = self._install()
        self.assertEqual(r2.returncode, 0, r2.stderr)
        self.assertTrue(self.link.is_symlink())
        self.assertEqual(
            os.readlink(self.link), str(self.dest / "bin/zeron-pet")
        )

    def test_reinstall_repoints_stale_managed_link_when_unchanged(self):
        r1 = self._install()
        self.assertEqual(r1.returncode, 0, r1.stderr)
        # The unchanged 9.9.9 tree stays, but the link now points at a
        # valid older managed install — install must repoint it to 9.9.9.
        old = self.pfx / "lib/iskra-desktop/9.9.8"
        (old / "bin").mkdir(parents=True)
        (old / ".iskra-desktop-install").write_text("9.9.8")
        self.link.unlink()
        self.link.symlink_to(old / "bin/zeron-pet")
        r2 = self._install()
        self.assertEqual(r2.returncode, 0, r2.stderr)
        self.assertEqual(
            os.readlink(self.link), str(self.dest / "bin/zeron-pet")
        )
        self.assertTrue(old.is_dir())

    def test_installed_script_repairs_missing_link(self):
        r1 = self._install()
        self.assertEqual(r1.returncode, 0, r1.stderr)
        self.link.unlink()
        r2 = subprocess.run(
            ["sh", str(self.dest / "install.sh"), "--prefix", str(self.pfx)],
            capture_output=True, text=True, env=self.env,
        )
        self.assertEqual(r2.returncode, 0, r2.stderr)
        self.assertTrue(self.link.is_symlink())
        self.assertEqual(
            os.readlink(self.link), str(self.dest / "bin/zeron-pet")
        )
        # Every installed file preserved; nothing backed up or removed.
        for rel in ("bin/zeron-pet", "assets/v6/manifest.json",
                    "install.sh", ".iskra-desktop-install"):
            self.assertTrue((self.dest / rel).is_file(), rel)
        self.assertFalse(
            (self.pfx / "lib/iskra-desktop/.backups").exists()
        )

    def test_installed_script_foreign_link_refused(self):
        r1 = self._install()
        self.assertEqual(r1.returncode, 0, r1.stderr)
        foreign = self.tmp / "foreign/bin/zeron-pet"
        foreign.parent.mkdir(parents=True)
        foreign.write_text("x")
        self.link.unlink()
        self.link.symlink_to(foreign)
        r2 = subprocess.run(
            ["sh", str(self.dest / "install.sh"), "--prefix", str(self.pfx)],
            capture_output=True, text=True, env=self.env,
        )
        self.assertNotEqual(r2.returncode, 0)
        self.assertTrue(self.link.is_symlink())
        self.assertEqual(os.readlink(self.link), str(foreign))
        self.assertTrue((self.dest / "bin/zeron-pet").is_file())
        self.assertFalse(
            (self.pfx / "lib/iskra-desktop/.backups").exists()
        )

    def test_first_install_link_failure_leaves_nothing(self):
        env = dict(self.env, ISKRA_DESKTOP_TEST_FAIL_AT="link")
        r = self._install(env=env)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.dest.exists())
        self.assertFalse(self.link.exists() or self.link.is_symlink())
        lib = self.pfx / "lib/iskra-desktop"
        self.assertFalse(list(lib.glob(".stage-*")))
        self.assertFalse((lib / ".backups").exists())

    def test_changed_dest_extra_file_survives_in_backup(self):
        r1 = self._install()
        self.assertEqual(r1.returncode, 0, r1.stderr)
        sentinel = self.dest / "user-notes.txt"
        sentinel.write_text("mine")
        readme = self.pkg / "README.md"
        orig = readme.read_bytes()
        readme.write_text("changed readme")
        try:
            r2 = self._install()
            self.assertEqual(r2.returncode, 0, r2.stderr)
            bks = list(
                (self.pfx / "lib/iskra-desktop/.backups").glob("9.9.9-*")
            )
            self.assertEqual(len(bks), 1)
            self.assertEqual(
                (bks[0] / "9.9.9/user-notes.txt").read_text(), "mine"
            )
            self.assertTrue(
                (bks[0] / "9.9.9/.iskra-desktop-install").is_file()
            )
            self.assertEqual(
                (self.dest / "README.md").read_text(), "changed readme"
            )
            self.assertFalse((self.dest / "user-notes.txt").exists())
            self.assertEqual(
                os.readlink(self.link), str(self.dest / "bin/zeron-pet")
            )
        finally:
            readme.write_bytes(orig)

    def test_rollback_restores_dest_and_link(self):
        r1 = self._install()
        self.assertEqual(r1.returncode, 0, r1.stderr)
        old_readme = (self.dest / "README.md").read_bytes()
        readme = self.pkg / "README.md"
        orig = readme.read_bytes()
        readme.write_text("v2 content")
        try:
            env = dict(self.env, ISKRA_DESKTOP_TEST_FAIL_AT="link")
            r2 = self._install(env=env)
            self.assertNotEqual(r2.returncode, 0)
            # Previous tree restored (old content), backup consumed.
            self.assertEqual(
                (self.dest / "README.md").read_bytes(), old_readme
            )
            self.assertTrue(self.link.is_symlink())
            self.assertEqual(
                os.readlink(self.link), str(self.dest / "bin/zeron-pet")
            )
        finally:
            readme.write_bytes(orig)

    def test_rollback_before_dest_install(self):
        r1 = self._install()
        self.assertEqual(r1.returncode, 0, r1.stderr)
        old_readme = (self.dest / "README.md").read_bytes()
        readme = self.pkg / "README.md"
        orig = readme.read_bytes()
        readme.write_text("v2 content")
        try:
            env = dict(self.env, ISKRA_DESKTOP_TEST_FAIL_AT="dest")
            r2 = self._install(env=env)
            self.assertNotEqual(r2.returncode, 0)
            self.assertEqual(
                (self.dest / "README.md").read_bytes(), old_readme
            )
            # No stray staging dirs left.
            self.assertFalse(
                list((self.pfx / "lib/iskra-desktop").glob(".stage-*"))
            )
        finally:
            readme.write_bytes(orig)

    def test_old_managed_link_repointed(self):
        # Simulate an older version already installed and linked.
        old = self.pfx / "lib/iskra-desktop/9.9.8"
        (old / "bin").mkdir(parents=True)
        (old / ".iskra-desktop-install").write_text("9.9.8")
        (self.pfx / "bin").mkdir(parents=True)
        self.link.symlink_to(old / "bin/zeron-pet")
        r = self._install()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(old.is_dir())  # old files preserved
        self.assertEqual(
            os.readlink(self.link),
            str(self.dest / "bin/zeron-pet"),
        )

    def test_foreign_prefix_link_refused(self):
        other = self.tmp / "other/lib/iskra-desktop/9.9.8"
        (other / "bin").mkdir(parents=True)
        (other / ".iskra-desktop-install").write_text("9.9.8")
        (self.pfx / "bin").mkdir(parents=True)
        self.link.symlink_to(other / "bin/zeron-pet")
        r = self._install()
        self.assertNotEqual(r.returncode, 0)
        self.assertTrue(self.link.is_symlink())
        self.assertFalse(self.dest.exists())

    def test_managed_link_without_marker_refused(self):
        old = self.pfx / "lib/iskra-desktop/9.9.8"
        (old / "bin").mkdir(parents=True)  # no marker file
        (self.pfx / "bin").mkdir(parents=True)
        self.link.symlink_to(old / "bin/zeron-pet")
        r = self._install()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(self.dest.exists())


class LicenseStageTests(unittest.TestCase):
    """lib_licenses.stage_licenses: validate fully, then copy — never
    mutate the source, never touch an existing destination."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="iskra-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.repo = self.tmp / "repo"
        fixture_repo(self.repo, with_plugin=False)
        self.snap = self.repo / "third_party/rust-licenses"
        self.lock = self.repo / "Cargo.lock"
        self.dest = self.tmp / "out/licenses"

    def _manifest(self):
        return json.loads((self.snap / "manifest.json").read_text())

    def _write_manifest(self, m):
        (self.snap / "manifest.json").write_text(json.dumps(m))

    def _set_files(self, files):
        m = self._manifest()
        m["packages"][0]["files"] = files
        self._write_manifest(m)

    def _patch_package(self, **kw):
        m = self._manifest()
        m["packages"][0].update(kw)
        self._write_manifest(m)

    def _refused(self):
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(self.snap, self.dest, self.lock)
        self.assertFalse(self.dest.exists())

    def test_valid_staging_exact_bytes(self):
        lib_licenses.stage_licenses(self.snap, self.dest, self.lock)
        src_files = sorted(
            p.relative_to(self.snap).as_posix()
            for p in self.snap.rglob("*")
            if p.is_file()
        )
        dst_files = sorted(
            p.relative_to(self.dest).as_posix()
            for p in self.dest.rglob("*")
            if p.is_file()
        )
        self.assertEqual(dst_files, src_files)
        for rel in src_files:
            self.assertEqual(
                (self.dest / rel).read_bytes(),
                (self.snap / rel).read_bytes(),
            )
        # source untouched: still valid for a second (fresh) destination
        lib_licenses.stage_licenses(
            self.snap, self.tmp / "out2/licenses", self.lock
        )

    def test_missing_snapshot_refused(self):
        shutil.rmtree(self.snap)
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(self.snap, self.dest, self.lock)
        self.assertFalse(self.dest.exists())

    def test_lock_mismatch_refused(self):
        self.lock.write_bytes(b"[other lock]\n")
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(self.snap, self.dest, self.lock)
        self.assertFalse(self.dest.exists())

    def test_changed_text_checksum_refused(self):
        lic = self.snap / "demo-crate-1.0.0/LICENSE"
        lic.write_bytes(b"tampered\n")
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(self.snap, self.dest, self.lock)
        self.assertFalse(self.dest.exists())

    def test_extra_file_refused(self):
        (self.snap / "extra.txt").write_text("x")
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(self.snap, self.dest, self.lock)
        self.assertFalse(self.dest.exists())

    def test_missing_declared_file_refused(self):
        (self.snap / "demo-crate-1.0.0/LICENSE").unlink()
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(self.snap, self.dest, self.lock)
        self.assertFalse(self.dest.exists())

    def test_path_traversal_refused(self):
        m = self._manifest()
        m["packages"][0]["files"][0]["path"] = "../escape"
        (self.snap / "manifest.json").write_text(json.dumps(m))
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(self.snap, self.dest, self.lock)
        self.assertFalse(self.dest.exists())

    def test_wrong_package_dir_refused(self):
        m = self._manifest()
        m["packages"][0]["files"][0]["path"] = "other-9.9.9/LICENSE"
        (self.snap / "manifest.json").write_text(json.dumps(m))
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(self.snap, self.dest, self.lock)

    def test_symlink_source_refused(self):
        victim = self.snap / "link"
        victim.symlink_to(self.tmp / "outside")
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(self.snap, self.dest, self.lock)
        self.assertTrue(victim.is_symlink())
        self.assertFalse(self.dest.exists())

    def test_preexisting_dest_refused(self):
        self.dest.mkdir(parents=True)
        (self.dest / "keep").write_text("x")
        before = {str(p) for p in self.snap.rglob("*")}
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(self.snap, self.dest, self.lock)
        self.assertEqual((self.dest / "keep").read_text(), "x")
        self.assertEqual(before, {str(p) for p in self.snap.rglob("*")})

    def test_malformed_manifest_types_refused(self):
        # Every malformed JSON shape fails as LicenseError, never as an
        # incidental AttributeError/TypeError.
        for bad in (
            [],                      # top level not an object
            "x",
            42,
            {"schema": 1, "cargo_lock_sha256": "x", "packages": {}},
            {"schema": 1, "cargo_lock_sha256": "x", "packages": ["x"]},
            {"schema": 1, "cargo_lock_sha256": "x", "packages": [42]},
            {"schema": 1, "cargo_lock_sha256": "x",
             "packages": [{"name": "p", "version": "1", "files": {}}]},
            {"schema": 1, "cargo_lock_sha256": "x",
             "packages": [{"name": "p", "version": "1",
                           "files": ["x"]}]},
            {"schema": 1, "cargo_lock_sha256": "x",
             "packages": [{"name": "p", "version": "1",
                           "files": [{"path": 7, "sha256": "y"}]}]},
        ):
            with self.subTest(bad=bad):
                # Pin the lockfile hash so validation reaches the shape
                # checks rather than stopping at "stale".
                if isinstance(bad, dict):
                    bad = dict(bad)
                    bad["cargo_lock_sha256"] = hashlib.sha256(
                        self.lock.read_bytes()
                    ).hexdigest()
                (self.snap / "manifest.json").write_text(json.dumps(bad))
                self._refused()

    def test_unsafe_package_components_refused(self):
        for patch in (
            {"name": "a/b"}, {"name": ".."}, {"name": ""},
            {"name": "a\\b"}, {"name": 7}, {"name": None},
            {"version": "1/0"}, {"version": "."}, {"version": ""},
            {"version": 1.0},
        ):
            with self.subTest(patch=patch):
                self._patch_package(**patch)
                self._refused()

    def test_unsafe_declared_paths_refused(self):
        sha = "a" * 64
        for bad_path in (
            "/abs/LICENSE",
            "demo-crate-1.0.0//LICENSE",
            "demo-crate-1.0.0/./LICENSE",
            "demo-crate-1.0.0/../demo-crate-1.0.0/LICENSE",
            "demo-crate-1.0.0\\LICENSE",
            "demo-crate-1.0.0/LICENSE/",
            "demo-crate-1.0.0",
            "",
            None,
            7,
        ):
            with self.subTest(bad_path=bad_path):
                self._set_files([{"path": bad_path, "sha256": sha}])
                self._refused()

    def test_bad_sha256_shape_refused(self):
        good_path = "demo-crate-1.0.0/LICENSE"
        for bad_sha in (
            "a" * 63, "a" * 65, "A" * 64, "g" * 64, "", None, 0,
            "a" * 32,
        ):
            with self.subTest(bad_sha=bad_sha):
                self._set_files([{"path": good_path, "sha256": bad_sha}])
                self._refused()

    def test_duplicate_package_id_refused(self):
        m = self._manifest()
        m["packages"].append(dict(m["packages"][0]))
        self._write_manifest(m)
        self._refused()

    def test_duplicate_declared_path_globally_refused(self):
        entry = dict(self._manifest()["packages"][0]["files"][0])
        self._set_files([entry, dict(entry)])
        self._refused()

    def test_canonical_alias_refused(self):
        # A path spelling that normalizes to a declared file is refused
        # before it can collide on disk.
        for alias in (
            "demo-crate-1.0.0/./LICENSE",
            "demo-crate-1.0.0/sub/../LICENSE",
            "demo-crate-1.0.0//LICENSE",
        ):
            with self.subTest(alias=alias):
                m = self._manifest()
                m["packages"][0]["files"].append(
                    {"path": alias, "sha256": "b" * 64}
                )
                self._write_manifest(m)
                self._refused()

    def test_symlink_snapshot_root_refused(self):
        real = self.tmp / "real-snap"
        self.snap.rename(real)
        link = self.tmp / "snap-link"
        link.symlink_to(real)
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(link, self.dest, self.lock)
        self.assertFalse(self.dest.exists())
        self.assertTrue(link.is_symlink())
        self.assertTrue((real / "manifest.json").is_file())

    def test_dest_inside_snapshot_refused(self):
        inner = self.snap / "inner"
        before = {str(p) for p in self.snap.rglob("*")}
        with self.assertRaises(lib_licenses.LicenseError):
            lib_licenses.stage_licenses(self.snap, inner, self.lock)
        self.assertFalse(inner.exists())
        self.assertEqual(before, {str(p) for p in self.snap.rglob("*")})


class StagePluginTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="iskra-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.plugins = self.tmp / "plugins"
        self.plugins.mkdir()
        self.dist = self.tmp / "dist/iskra-0.1.0"
        self.dist.mkdir(parents=True)
        (self.dist / "zeron-plugin.json").write_text(
            json.dumps({"id": "iskra", "version": "0.1.0",
                        "entry": "init.luau"})
        )
        (self.dist / "init.luau").write_text("return {}")
        (self.dist / "zeron.luau").write_text("-- sdk")

    def _stage(self):
        return subprocess.run(
            ["sh", str(SCRIPTS / "stage-plugin.sh"),
             "--dist", str(self.dist),
             "--plugins-dir", str(self.plugins)],
            capture_output=True, text=True,
        )

    def test_stage_backs_up_outside_plugins_root(self):
        live = self.plugins / "iskra"
        live.mkdir()
        (live / "old.txt").write_text("old")
        r = self._stage()
        self.assertEqual(r.returncode, 0, r.stderr)
        # Plugins root holds ONLY the new folder — no iskra.bak-* inside.
        self.assertEqual(
            sorted(p.name for p in self.plugins.iterdir()), ["iskra"]
        )
        bks = list((self.tmp / "plugin-backups/iskra").glob("iskra-*"))
        self.assertEqual(len(bks), 1)
        self.assertEqual((bks[0] / "old.txt").read_text(), "old")
        self.assertTrue((live / "init.luau").is_file())

    def test_signed_dist_refused(self):
        m = json.loads((self.dist / "zeron-plugin.json").read_text())
        m["signature"] = {"key": "x", "files": {}, "sig": "y"}
        (self.dist / "zeron-plugin.json").write_text(json.dumps(m))
        r = self._stage()
        self.assertNotEqual(r.returncode, 0)

    def test_wrong_id_refused(self):
        (self.dist / "zeron-plugin.json").write_text(
            json.dumps({"id": "other", "version": "0.1.0"})
        )
        r = self._stage()
        self.assertNotEqual(r.returncode, 0)

    def test_symlink_in_dist_refused(self):
        (self.dist / "evil").symlink_to("/etc/passwd")
        r = self._stage()
        self.assertNotEqual(r.returncode, 0)

    def test_failed_backup_leaves_live_untouched(self):
        live = self.plugins / "iskra"
        live.mkdir()
        (live / "old.txt").write_text("old")
        # Plugins root stays writable (staging must succeed); the backup
        # PARENT can't be created — the failure hits the backup path
        # itself, not the earlier staging mktemp.
        os.chmod(self.tmp, 0o500)
        try:
            r = self._stage()
        finally:
            os.chmod(self.tmp, 0o700)
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual((live / "old.txt").read_text(), "old")


if __name__ == "__main__":
    unittest.main(verbosity=2)
