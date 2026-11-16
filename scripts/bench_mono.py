#!/usr/bin/env python3
"""Compare two release rockc binaries on many retained trait-impl candidates.

Only one impl is called. Source reachability must keep the candidate metadata;
demand-driven mono should materialize only the selected member.
"""

import argparse
import os
from pathlib import Path
import re
import statistics
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--impls", type=int, default=256)
    parser.add_argument("--repeats", type=int, default=5)
    args = parser.parse_args()
    if args.impls < 1 or args.repeats < 1:
        parser.error("--impls and --repeats must be positive")
    root = args.output_dir.resolve()
    root.mkdir(parents=True, exist_ok=True)
    entry = root / "mono_candidates.rk"
    entry.write_text(
        "trait Value\n    @get: I64\n"
        + "".join(
            f"struct Item{n}\n    < value: I64\nimpl Value for Item{n}\n    @get = -> @value\n"
            for n in range(args.impls)
        )
        + "main = ->\n    item = Item0\n        value: 42\n    item.get!\n"
    )
    samples = {"baseline": [], "candidate": []}
    # Alternate compilers to reduce drift from load and CPU frequency changes.
    for repeat in range(args.repeats):
        for label, compiler in [("baseline", args.baseline), ("candidate", args.candidate)]:
            output = root / f"{label}-{repeat}"
            result = subprocess.run(
                [str(compiler.resolve()), "--entry-file", str(entry), "--output-dir", str(output),
                 "--no-std", "--no-prelude", "--no-link", "--emit-llvm", "-A", "unused"],
                env={**os.environ, "ROCK_TIME_PHASES": "1"},
                capture_output=True, text=True,
            )
            (root / f"{label}-{repeat}.log").write_text(result.stdout + result.stderr)
            result.check_returncode()
            phases = {
                phase: float(value)
                for phase, value in re.findall(r"\[rock-time\] (\S+)\s+([\d.]+) ms", result.stderr)
            }
            if "emit" not in phases:
                raise RuntimeError(f"{compiler} did not report completed phase timings")
            samples[label].append(phases)
    for label, runs in samples.items():
        median = lambda phase: statistics.median(run[phase] for run in runs)
        total = statistics.median(sum(run.values()) for run in runs)
        print(f"{label}: total={total:.3f} ms, mono={median('mono'):.3f} ms, "
              f"mir-bodies={median('mir-bodies'):.3f} ms")


if __name__ == "__main__":
    main()
