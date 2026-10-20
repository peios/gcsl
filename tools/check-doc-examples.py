#!/usr/bin/env python3
"""Execute authored GCSL examples and check the tutorial's expected results."""

import os
import re
import subprocess
import tempfile
import textwrap
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parent.parent
LEARN = REPOSITORY / "docs/learn/2--developer-tools.shelf/gcsl.product"
WORKSPACE_PAGE = "200--organizing-a-workspace.md"
EXPECTED = {
    ("200--running-a-first-program.md", 0): "gcsl 0.1.0\n",
    ("100--values-and-bindings.md", 1): "16\n",
    ("200--records-and-types.md", 0): "[]\n",
    ("200--records-and-types.md", 2): "8080\n",
    ("300--functions-and-effects.md", 0): "42\n",
    ("400--deferred-work.md", 0): "42\n",
    ("500--lists-and-control-flow.md", 0): "8\n",
    ("500--lists-and-control-flow.md", 1): "[4, 6]\n",
    ("500--lists-and-control-flow.md", 2): "[0, 2, 4]\n",
    ("300--file-and-process-effects.md", 0): "GCSL\n",
}


def examples(page: Path) -> list[str]:
    """Read complete fenced GCSL samples, removing procedure indentation."""
    return [
        textwrap.dedent(body) + "\n"
        for body in re.findall(r"```gcsl\n(.*?)\n\s*```", page.read_text(), re.DOTALL)
    ]


def run(binary: str, directory: Path, *arguments: str) -> subprocess.CompletedProcess[str]:
    """Require a successful example command and retain its output for assertions."""
    result = subprocess.run(
        [binary, *arguments],
        cwd=directory,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(f"{' '.join(arguments)} failed:\n{result.stdout}{result.stderr}")
    return result


def check_standalone(binary: str, directory: Path) -> int:
    """Run every standalone sample and validate explicitly documented values."""
    count = 0
    seen = set()
    for page in sorted(LEARN.rglob("*.md")):
        if page.name == WORKSPACE_PAGE:
            continue
        for number, source in enumerate(examples(page)):
            path = directory / f"{page.stem}-{number}.gcsl"
            path.write_text(source)
            result = run(binary, directory, "run", str(path))
            if result.stderr:
                raise RuntimeError(f"{page.name} sample {number}: {result.stderr}")
            key = (page.name, number)
            if key in EXPECTED:
                if result.stdout != EXPECTED[key]:
                    raise RuntimeError(f"{key}: expected {EXPECTED[key]!r}, got {result.stdout!r}")
                seen.add(key)
            count += 1
    if seen != EXPECTED.keys():
        raise RuntimeError(f"Documented examples disappeared: {EXPECTED.keys() - seen}")
    return count


def check_workspace(binary: str, directory: Path) -> None:
    """Check the multifile tutorial and its documented summary and value."""
    page = next(LEARN.rglob(WORKSPACE_PAGE))
    sources = examples(page)
    if len(sources) != 2:
        raise RuntimeError("Workspace tutorial needs its two source files")
    directory.mkdir()
    for name, source in zip(("workspace.gcsl", "main.gcsl"), sources, strict=True):
        (directory / name).write_text(source)
    checked = run(binary, directory, "check")
    if checked.stderr != "no problems in 2 files checked\n" or checked.stdout:
        raise RuntimeError(f"Unexpected workspace check result: {checked}")
    result = run(binary, directory, "run", "main.gcsl")
    if result.stdout != "gcsl 0.1.0\n" or result.stderr:
        raise RuntimeError(f"Unexpected workspace run result: {result}")
    run(binary, directory, "fmt", "main.gcsl")
    formatted = run(binary, directory, "fmt", "--check", "main.gcsl")
    if formatted.stdout or formatted.stderr:
        raise RuntimeError(f"Unexpected formatting check result: {formatted}")


def main() -> None:
    """Check temporary copies, leaving authored source untouched."""
    binary = os.environ.get("GCSL", "gcsl")
    if "/" in binary:
        binary = str(Path(binary).resolve())
    with tempfile.TemporaryDirectory(prefix="gcsl-doc-examples-") as temporary:
        directory = Path(temporary)
        count = check_standalone(binary, directory)
        check_workspace(binary, directory / "workspace")
    print(f"GCSL docs: {count} standalone examples and the workspace procedure passed")


if __name__ == "__main__":
    main()
