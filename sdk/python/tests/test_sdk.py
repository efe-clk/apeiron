"""What the Python SDK adds on top of the C ABI."""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

import apeiron  # noqa: E402


class Sdk(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "src").mkdir()
        (self.root / "src" / "a.txt").write_text("first")

    def tearDown(self):
        self.tmp.cleanup()

    def test_typed_methods_records_and_jobs(self):
        with apeiron.Repository.init(self.root / "repo") as repo:
            job = repo.backup(self.root / "src", message="one")
            phases = [p.phase for p in job.watch(interval=0.01)]
            self.assertTrue(all(isinstance(p, str) for p in phases))
            report = job.result()
            self.assertEqual(report.files, 1)
            self.assertIs(job.wait(), report)
            self.assertEqual(repo.resolve("main"), report.snapshot)
            self.assertEqual(repo.log().snapshots[0].message, "one")
            self.assertEqual([e.name for e in repo.list("main")], ["a.txt"])

            repo.create_ref("feature")
            (self.root / "src" / "a.txt").write_text("second")
            repo.backup(self.root / "src", ref="feature").wait()
            self.assertEqual(repo.diff("main", "feature")[0].kind, "modified")
            self.assertEqual(repo.merge("feature").action, "fast_forward")
            hits = repo.search("*.TXT", history=True).wait().hits
            self.assertEqual([h.path for h in hits], ["a.txt", "a.txt"])

            with self.assertRaises(AttributeError):
                report.missing

    def test_errors_are_exceptions_by_kind(self):
        with apeiron.Repository.init(self.root / "repo") as repo:
            with self.assertRaises(apeiron.NotFoundError) as caught:
                repo.resolve("main")
            self.assertEqual((caught.exception.code, caught.exception.kind), (2, "not_found"))
            self.assertIsInstance(caught.exception, apeiron.ApeironError)
            with self.assertRaises(apeiron.UnsupportedError):
                repo.call("nonsense")
            repo.backup(self.root / "src").wait()
            job = repo.search()
            with self.assertRaises(apeiron.InvalidInputError):
                job.wait()
            # The error is kept, as a result would be.
            with self.assertRaises(apeiron.InvalidInputError):
                job.result()
        with self.assertRaises(apeiron.InvalidInputError):
            repo.refs()

    def test_wrong_password(self):
        repo, key = apeiron.Repository.init_encrypted(self.root / "repo", "pw")
        repo.close()
        self.assertGreater(len(key), 20)
        with self.assertRaises(apeiron.PasswordError):
            apeiron.Repository.open(self.root / "repo", "wrong")
        with apeiron.Repository.open(self.root / "repo", key) as repo:
            self.assertTrue(repo.info().encrypted)

    def test_plugins_without_a_repository(self):
        folder = self.root / "plugins"
        self.assertEqual(apeiron.plugins(folder), [])
        with self.assertRaises(apeiron.NotFoundError):
            apeiron.remove_plugin("missing", folder)
        with self.assertRaises(apeiron.InvalidInputError):
            apeiron.add_plugin(self.root / "src" / "a.txt", folder)

    def test_conflicts_without_a_repository(self):
        folder = self.root / "synced"
        folder.mkdir()
        (folder / "a.txt").write_text("here")
        (folder / "a (conflict pc 2026-10-31 10-08).txt").write_text("there")
        found = apeiron.conflicts(folder)
        self.assertEqual([(c.path, c.exists) for c in found], [("a.txt", True)])
        apeiron.resolve_conflict(folder, found[0]["copy"], "copy")
        self.assertEqual((folder / "a.txt").read_text(), "there")
        self.assertEqual(apeiron.conflicts(folder), [])

    def test_clone(self):
        with apeiron.Repository.init(self.root / "repo") as repo:
            repo.backup(self.root / "src").wait()
        report = apeiron.Repository.clone(self.root / "repo", self.root / "copy").wait()
        self.assertEqual(report.refs[0].name, "main")
        with apeiron.Repository.open(self.root / "copy") as copy:
            self.assertEqual(copy.remotes()[0].name, "origin")


if __name__ == "__main__":
    unittest.main()
