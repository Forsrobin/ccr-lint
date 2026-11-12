#!/usr/bin/env python3
"""Record commitlint's output (`commitlint --edit <file>`, colors off) for every message of
tests/corpus/lint-cases.txt into tests/corpus/lint-expected.json, the golden data of tests/lint_golden.rs.

    NODE_PROJECT=/path/to/project/with/commitlint scripts/gen_lint_golden.py
"""
import json, os, subprocess, tempfile
from concurrent.futures import ThreadPoolExecutor

here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
cases = open(os.path.join(here, "tests/corpus/lint-cases.txt"), encoding="utf-8").read().split("\0")
project = os.environ["NODE_PROJECT"]
env = {**os.environ, "NO_COLOR": "1"}
env.pop("CI", None); env.pop("FORCE_COLOR", None)

def run(arg):
    i, msg = arg
    with tempfile.NamedTemporaryFile("w", suffix=".msg", delete=False, encoding="utf-8") as f:
        f.write(msg)
    r = subprocess.run(["npx", "--no-install", "commitlint", "--edit", f.name], cwd=project, env=env,
                       capture_output=True, text=True)
    os.unlink(f.name)
    return {"message": msg, "stdout": r.stdout, "exit": r.returncode}

with ThreadPoolExecutor(8) as ex:
    out = list(ex.map(run, enumerate(cases)))
json.dump(out, open(os.path.join(here, "tests/corpus/lint-expected.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print(len(out), "cases recorded")
