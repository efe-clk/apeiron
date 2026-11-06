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
            self.assertEqual(len(repo.log(grep="ONE").snapshots), 1)
            self.assertEqual(repo.log(grep="nothing").snapshots, [])
            self.assertEqual(repo.duplicates(), [])
            self.assertEqual([e.name for e in repo.list("main")], ["a.txt"])

            repo.create_ref("feature")
            (self.root / "src" / "a.txt").write_text("second")
            repo.backup(self.root / "src", ref="feature").wait()
            self.assertEqual(repo.diff("main", "feature")[0].kind, "modified")
            self.assertEqual(repo.merge("feature").action, "fast_forward")
            self.assertEqual(repo.cherry_pick("feature").action, "up_to_date")
            self.assertEqual(repo.rebase("feature").action, "up_to_date")
            work = self.root / "work"
            self.assertEqual(len(repo.switch("main", work).wait().changes), 1)
            self.assertEqual((work / "a.txt").read_text(), "second")
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

    def test_legal_holds(self):
        with apeiron.Repository.init(self.root / "repo") as repo:
            repo.backup(self.root / "src").wait()
            hold = repo.add_hold("main", "Case 1")
            with self.assertRaises(apeiron.ConflictError):
                repo.delete_ref("main", repo.resolve("main"))
            lifted = repo.lift_hold(hold.id, "Closed")
            self.assertEqual(lifted.lift.reason, "Closed")
            self.assertEqual(len(repo.holds()), 1)

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

    def test_saved_searches_without_a_repository(self):
        file = self.root / "searches.json"
        self.assertEqual(apeiron.saved_searches(file), [])
        apeiron.save_search("photos", "*.jpg", file, history=True)
        found = apeiron.saved_searches(file)
        self.assertEqual([(s.name, s.pattern, s.history) for s in found], [("photos", "*.jpg", True)])
        self.assertEqual(apeiron.forget_search("photos", file).name, "photos")
        with self.assertRaises(apeiron.NotFoundError):
            apeiron.forget_search("photos", file)
        with self.assertRaises(apeiron.InvalidInputError):
            apeiron.save_search("empty", file=file)

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
            self.assertEqual(copy.pull("origin", rate=1 << 30).wait().objects, 0)
            with self.assertRaises(apeiron.InvalidInputError):
                copy.push("origin", rate=0).wait()

    def test_every_ref_and_the_report(self):
        with apeiron.Repository.init(self.root / "repo") as repo:
            repo.backup(self.root / "src").wait()
            repo.add_remote("disk", str(self.root / "disk"))
            pushed = repo.push_all("disk", rate=1 << 30).wait()
            self.assertEqual(pushed.refs[0].name, "main")
            pulled = repo.pull_all("disk").wait()
            self.assertEqual(pulled.refs[0].name, "main")
            report = repo.report(stale_days=0).wait()
            self.assertEqual(report.refs[0].name, "main")


if __name__ == "__main__":
    unittest.main()
