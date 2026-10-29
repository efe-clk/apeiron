"""The shared conformance scenarios (docs/SDK.md §6) run through Python."""

import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "src"))

import apeiron  # noqa: E402

SCENARIOS = HERE.parent.parent / "conformance"


def holds(actual, expected):
    """Whether ``actual`` holds everything in ``expected``: objects by key,
    lists element by element with the same length, other values by equality."""
    if isinstance(expected, dict):
        return isinstance(actual, dict) and all(
            k in actual and holds(actual[k], v) for k, v in expected.items()
        )
    if isinstance(expected, list):
        return (
            isinstance(actual, list)
            and len(actual) == len(expected)
            and all(holds(a, e) for a, e in zip(actual, expected))
        )
    return actual == expected


def expand(value, tmp):
    if isinstance(value, str):
        return value.replace("$TMP", tmp)
    if isinstance(value, list):
        return [expand(v, tmp) for v in value]
    if isinstance(value, dict):
        return {k: expand(v, tmp) for k, v in value.items()}
    return value


def run(test, file):
    scenario = json.loads(file.read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        repo = None
        try:
            for i, step in enumerate(scenario["steps"], 1):
                step = expand(step, tmp)
                at = f"{file.name} step {i}: {step}"
                request = step.get("request", {})
                password = step.get("password")
                if "write" in step:
                    path = root / step["write"]
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(step["text"].encode())
                    continue
                if "remove" in step:
                    (root / step["remove"]).unlink()
                    continue
                if "read" in step:
                    test.assertEqual((root / step["read"]).read_bytes().decode(), step["text"], at)
                    continue
                if "init" in step:
                    if repo:
                        repo.close()
                    if password is None:
                        repo = apeiron.Repository.init(root / step["init"])
                    else:
                        repo, key = apeiron.Repository.init_encrypted(root / step["init"], password)
                        test.assertTrue(key, at)
                    continue
                try:
                    if "open" in step:
                        opened = apeiron.Repository.open(root / step["open"], password)
                        if repo:
                            repo.close()
                        repo, result = opened, None
                    elif "call" in step:
                        result = repo.call(step["call"], **request)
                    elif "start" in step:
                        with repo.start(step["start"], **request) as job:
                            result = job.wait()
                    else:
                        test.fail(f"{at}: unknown step")
                except apeiron.ApeironError as e:
                    test.assertEqual(e.kind, step.get("error"), f"{at}: {e}")
                    continue
                test.assertNotIn("error", step, f"{at}: expected an error, got {result}")
                if "expect" in step:
                    test.assertTrue(holds(result, step["expect"]), f"{at}\ngot {result}")
        finally:
            if repo:
                repo.close()


class Conformance(unittest.TestCase):
    def test_every_scenario(self):
        files = sorted(SCENARIOS.glob("*.json"))
        self.assertGreaterEqual(len(files), 4)
        for file in files:
            with self.subTest(file.name):
                run(self, file)


if __name__ == "__main__":
    unittest.main()
