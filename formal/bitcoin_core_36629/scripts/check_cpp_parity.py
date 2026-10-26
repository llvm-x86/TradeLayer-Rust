#!/usr/bin/env python3
"""Execute exact pinned Bitcoin Core formatter header and compare to Lean 4.

Models only serializer byte IO via cpp_stubs/serialize.h.
This is sampled source parity; universal equivalence of C++ and Lean
implementations still requires a refinement proof.
"""
from pathlib import Path
import subprocess
import tempfile

HERE = Path(__file__).resolve().parents[1]
PINNED_BLOB = "fe7cf1f5239aea3bf98f301c12953a0c8f402a8e"
header = HERE / "upstream" / "ordered_varint.h"
result = subprocess.run(
    ["git", "hash-object", str(header)], check=True, text=True,
    capture_output=True
)
assert result.stdout.strip() == PINNED_BLOB, "Pinned upstream header Git blob differs"
lean = subprocess.run(
    ["lake", "env", "lean", "--run", str(HERE / "lean_oracle.lean")],
    cwd=HERE, check=True, text=True, capture_output=True,
).stdout
rows = [line for line in lean.splitlines() if line.strip()]
assert len(rows) > 4000, len(rows)
numbers = [row.split(" ", 1)[0] for row in rows]
with tempfile.TemporaryDirectory() as temp:
    target = Path(temp) / "oracle"
    subprocess.run([
        "g++", "-std=c++20", "-O2", "-Wall", "-Wextra", "-Werror",
        "-I", str(HERE / "cpp_stubs"),
        "-I", str(HERE / "upstream"),
        str(HERE / "cpp_oracle.cpp"), "-o", str(target)
    ], check=True, cwd=HERE)
    cpp = subprocess.run(
        [str(target)], input="\n".join(numbers) + "\n",
        check=True, text=True, capture_output=True,
    )
actual = cpp.stdout.splitlines()
assert len(actual) == len(rows), (len(actual), len(rows))
for index, (real, model) in enumerate(zip(actual, rows)):
    if real != model:
        raise AssertionError(f"source parity failed on sample {index}: "
                             f"upstream={real!r}, Lean={model!r}")
print(f"PASS: {len(rows)} Lean/C++ encoding and decode samples, "
      f"plus C++ malformed-prefix, truncation and overflow rejection tests")
print(f"upstream_header_git_blob={PINNED_BLOB}")
