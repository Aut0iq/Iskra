"""Portable authoring, package integrity and topology contracts."""
import base64
import importlib.util
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import avatars


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / (name + '.py'))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


class AvatarTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        from PIL import Image
        image = Image.new('RGBA', (32, 48), '#cd3f78')
        image.save(self.root / 'portrait.png')
        self.pack = self.root / 'avatar'
        avatars.initialize(self.root / 'portrait.png', self.pack, 'test-avatar', 'Test', 'CC0-1.0', 'Fixture artwork')

    def tearDown(self):
        self.temp.cleanup()

    def test_portrait_builds_all_states_with_credit(self):
        meta, manifest = avatars.validate(self.pack)
        self.assertEqual(meta['author'], 'Test')
        self.assertEqual(len(manifest['animations']), 38)
        self.assertEqual(sum(len(a['frames']) for a in manifest['animations'].values()), 304)

    def test_pack_code_is_rejected(self):
        (self.pack / 'init.luau').write_text('untrusted')
        with self.assertRaises(ValueError):
            avatars.validate(self.pack)

    def test_path_traversal_rejected(self):
        path = self.pack / 'manifest.json'
        manifest = json.loads(path.read_text())
        manifest['animations']['standing_idle']['frames'][0] = '../outside.png'
        path.write_text(json.dumps(manifest))
        with self.assertRaises((ValueError, avatars.lib_assets.AssetError)):
            avatars.validate(self.pack)

    def test_author_required(self):
        path = self.pack / 'avatar.json'
        meta = json.loads(path.read_text()); meta['author'] = ''
        path.write_text(json.dumps(meta))
        with self.assertRaises(ValueError):
            avatars.validate(self.pack)

    def test_unrecognized_metadata_cannot_introduce_commands(self):
        path = self.pack / 'avatar.json'
        meta = json.loads(path.read_text()); meta['exec'] = 'untrusted-program'
        path.write_text(json.dumps(meta))
        with self.assertRaises(ValueError): avatars.validate(self.pack)

    def test_custom_plugin_carries_utf8_author_credit(self):
        path = self.pack / 'avatar.json'
        meta = json.loads(path.read_text()); meta['author'] = 'Лиса 雪'
        path.write_text(json.dumps(meta))
        repo = self.root / 'repo'
        shutil.copytree(ROOT / 'plugin/iskra', repo / 'plugin/iskra', ignore=shutil.ignore_patterns('assets', '.assets*'))
        for name in ('LICENSE', 'LICENSE-ARTWORK', 'ARTWORK.md', 'TRADEMARKS.md'):
            shutil.copy2(ROOT / name, repo / name)
        result = subprocess.run([sys.executable, str(ROOT / 'scripts/build-plugin.py'),
                                 '--repo-root', str(repo), '--avatar', str(self.pack),
                                 '--output', str(repo / 'dist'), '--wait', '0'], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        staged = next((repo / 'dist').glob('iskra-*')) / 'assets'
        self.assertIn('Лиса 雪', (staged / 'ARTWORK.md').read_text(encoding='utf-8'))
        self.assertEqual(json.loads((staged / 'avatar.json').read_text())['author'], 'Лиса 雪')

    def test_existing_output_preserved(self):
        with self.assertRaises(ValueError):
            avatars.initialize(self.root / 'portrait.png', self.pack, 'test', 'Test', 'CC0', 'Fixture')
        self.assertTrue((self.pack / 'avatar.json').is_file())

    def test_zip_is_deterministic_and_canonical(self):
        package = module('package-portable').package
        a, b = self.root / 'a.zip', self.root / 'b.zip'
        package(self.pack, a); package(self.pack, b)
        self.assertEqual(a.read_bytes(), b.read_bytes())
        with zipfile.ZipFile(a) as z:
            self.assertTrue(all(not n.startswith('/') and '..' not in Path(n).parts for n in z.namelist()))
            self.assertIn('avatar.json', z.namelist())


class PolicyTests(unittest.TestCase):
    def event(self, base, head, repo='Energy-Quant/Iskra', user='Energy-Quant'):
        return {'pull_request': {'base': {'ref': base}, 'head': {'ref': head, 'repo': {'full_name': repo, 'owner': {'login': repo.split('/')[0]}}}, 'user': {'login': user}}}

    def test_main_accepts_only_upstream_dev(self):
        check = module('branch-policy').validate
        check(self.event('main', 'dev'))
        for event in (self.event('main', 'feature-test'), self.event('main', 'dev', 'alice/Iskra', 'alice')):
            with self.assertRaises(ValueError): check(event)

    def test_fork_author_and_branch_convention(self):
        check = module('branch-policy').validate
        check(self.event('dev', 'feature-avatar', 'alice/Iskra', 'alice'))
        check(self.event('dev', 'bugfix-frame'))
        for event in (self.event('dev', 'random'), self.event('dev', 'feature-test', 'bob/Iskra', 'alice')):
            with self.assertRaises(ValueError): check(event)

    def test_pre_push_blocks_main_tags_and_unacknowledged_dev(self):
        hook = ROOT / '.githooks/pre-push'
        for ref in ('refs/heads/main', 'refs/tags/v9.9.9', 'refs/heads/dev'):
            env = dict(os.environ); env.pop('ISKRA_CONFIRM_DEV_PUSH', None)
            result = subprocess.run([sys.executable, str(hook)], input=f'HEAD abc {ref} def\n', text=True, capture_output=True, env=env)
            self.assertNotEqual(result.returncode, 0)


class SignatureTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform == 'linux', 'Ed25519 verification is mandatory in Linux CI')
    def test_published_signature_and_tamper_detection(self):
        verify = module('verify-signed-plugin').verify_signature
        manifest = json.loads((ROOT / 'tests/fixtures/published-v0.1.0-manifest.json').read_text())
        verify(manifest)
        manifest['name'] = 'Tampered'
        with self.assertRaises(ValueError): verify(manifest)


class DeliveryTests(unittest.TestCase):
    def test_no_release_from_failed_pr_or_fork_run(self):
        release = module('release-control')
        valid = {'conclusion': 'success', 'head_branch': 'main',
                 'head_repository': {'full_name': 'Energy-Quant/Iskra'},
                 'event': 'push', 'path': '.github/workflows/ci.yml'}
        self.assertEqual(release.source_run(valid, 'publish'), 'main')
        for changes in ({'conclusion': 'cancelled'}, {'conclusion': 'failure'},
                        {'event': 'pull_request'}, {'head_branch': 'feature-escape'},
                        {'head_repository': {'full_name': 'alice/Iskra'}},
                        {'path': '.github/workflows/unreviewed.yml'}):
            with self.assertRaises(ValueError):
                release.source_run(dict(valid, **changes), 'publish')
        self.assertEqual(release.source_run(dict(valid, head_branch='dev'), 'nightly'), 'dev')
        with self.assertRaises(ValueError):
            release.source_run(dict(valid, head_branch='dev'), 'prepare')

    def test_artifact_traversal_is_rejected_before_extraction(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); archive = root / 'malicious.zip'
            with zipfile.ZipFile(archive, 'w') as z:
                z.writestr('../escape', 'do not write')
            with self.assertRaises(ValueError):
                module('release-control').unzip(archive, root / 'dest')
            self.assertFalse((root / 'escape').exists())

    def test_rule_layers_keep_ci_from_bypassing_checks(self):
        configure = module('configure-repository')
        rules = {r['name']: r for r in configure.desired(123)}
        self.assertEqual(rules['main-required-review-and-ci']['bypass_actors'], [])
        self.assertEqual(rules['main-ci-updates-only']['bypass_actors'],
                         [{'actor_id': 123, 'actor_type': 'Integration', 'bypass_mode': 'always'}])
        self.assertEqual(rules['release-tags-immutable']['bypass_actors'], [])
        closed = {r['name']: r for r in configure.desired(0)}
        self.assertEqual(closed['release-tags-ci-creation']['bypass_actors'], [])


if __name__ == '__main__':
    unittest.main()
