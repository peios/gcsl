"""Validate staged GCSL executables, debugging data and manuals."""

import gzip
import os
import re
import subprocess
import tempfile
from pathlib import Path

SMOKE_TIMEOUT_SECONDS = 15

MANUALS = {"gcsl.1": "1", "gcsl.5": "5", "gcsl-stdlib.3": "3"}


def require(condition: bool, message: str) -> None:
    """Keep package gates effective when Python assertions are disabled."""
    if not condition:
        raise ValueError(message)


def readelf(path: Path, option: str) -> str:
    """Read one ELF view, tolerating binary CRC bytes in the debug-link section."""
    return subprocess.check_output(
        ["readelf", option, str(path)],
        text=True,
        errors="replace",
        env=os.environ | {"LC_ALL": "C"},
    )


def build_id(notes: str, path: Path) -> str:
    """Extract the single GNU build identity required by split debugging."""
    matches = re.findall(r"Build ID: ([0-9a-f]+)", notes)
    require(len(matches) == 1 and len(matches[0]) >= 3, f"{path}: missing or ambiguous build ID")
    return matches[0]


def validate_elf(stage: Path, binary: str, *, native: bool) -> None:
    """Require linked hardening and the matching shared-family split debug payload."""
    elf = stage / "usr/bin" / binary
    require(elf.is_file() and os.access(elf, os.X_OK), f"{elf}: missing executable")
    header, program, dynamic = (readelf(elf, option) for option in ("-h", "-lW", "-dW"))
    notes, sections = readelf(elf, "-n"), readelf(elf, "-SW")
    require(bool(re.search(r"Type:\s+DYN\b", header)), f"{elf}: not position independent")
    require("GNU_RELRO" in program and "BIND_NOW" in dynamic, f"{elf}: missing full RELRO")
    stack = [line for line in program.splitlines() if "GNU_STACK" in line]
    require(len(stack) == 1 and bool(re.search(r"\sRW\s", stack[0])), f"{elf}: executable stack")
    require(bool(re.search(r"FLAGS_1.*\bPIE\b", dynamic)), f"{elf}: missing PIE flag")
    needed = set(re.findall(r"Shared library: \[(.*?)\]", dynamic))
    require(
        needed <= {"libc.so.6", "libgcc_s.so.1", "libm.so.6", "ld-linux-x86-64.so.2"},
        f"{elf}: undeclared shared library dependencies: {needed}",
    )
    require(not re.search(r"\((RPATH|RUNPATH|TEXTREL)\)", dynamic), f"{elf}: unsafe dynamic tag")
    require(".debug_info" not in sections, f"{elf}: unsplit debug information")
    if native:
        require("IBT" in notes and "SHSTK" in notes, f"{elf}: missing native CET properties")
        require(bool(re.search(r"\(RELR\)", dynamic)), f"{elf}: missing native packed relocations")
    identity = build_id(notes, elf)
    debug = stage / f"debug/gcsl/usr/lib/debug/.build-id/{identity[:2]}/{identity[2:]}.debug"
    require(debug.is_file() and debug.stat().st_size > 0, f"{elf}: missing split debug file")
    require(".debug_info" in readelf(debug, "-SW"), f"{debug}: missing debug information")
    require(build_id(readelf(debug, "-n"), debug) == identity, f"{debug}: build ID mismatch")
    require(debug.name in readelf(elf, "--string-dump=.gnu_debuglink"), f"{elf}: wrong debug link")
    forbidden = {str(Path.cwd().resolve()), str(Path.cwd().resolve()).split("/execroot/")[0]}
    for path in (elf, debug):
        contents = path.read_bytes()
        for prefix in forbidden:
            require(len(prefix) <= 1 or prefix.encode() not in contents, f"{path}: leaked {prefix}")


def validate_data(stage: Path) -> None:
    """Require all manuals in their declared section and no mutable state."""
    for name, section in MANUALS.items():
        manual = stage / f"usr/share/man/man{section}/{name}.gz"
        require(manual.is_file(), f"missing manual: {manual}")
        with gzip.open(manual, "rt", encoding="utf-8") as stream:
            require(
                bool(re.search(r"^\.TH ", stream.read(), re.MULTILINE)),
                f"missing heading: {manual}",
            )
    require(not (stage / "var").exists(), "package unexpectedly contains mutable state")


def validate(stage: Path, *, native: bool) -> None:
    """Check hardening and run the staged interpreter against disposable input."""
    validate_elf(stage, "gcsl", native=native)
    validate_data(stage)
    binary = (stage / "usr/bin/gcsl").resolve()
    version = subprocess.check_output(
        [binary, "--version"], text=True, timeout=SMOKE_TIMEOUT_SECONDS
    )
    require(version.strip() == "gcsl 0.1.0", "unexpected executable version")
    with tempfile.TemporaryDirectory(prefix="gcsl-package-") as temporary:
        program = Path(temporary) / "smoke.gcsl"
        program.write_text("1 + 2\n", encoding="utf-8")
        checked = subprocess.run(
            [binary, "check", program],
            capture_output=True,
            text=True,
            timeout=SMOKE_TIMEOUT_SECONDS,
        )
        require(checked.returncode == 0, f"staged checker failed: {checked.stderr}")
        result = subprocess.run(
            [binary, "run", program], capture_output=True, text=True, timeout=SMOKE_TIMEOUT_SECONDS
        )
        require(
            result.returncode == 0 and result.stdout == "3\n" and not result.stderr,
            f"staged interpreter failed: {result}",
        )
