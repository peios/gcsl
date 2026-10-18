#!/usr/bin/python3
"""Stage and validate declared GCSL artifacts before writing any PPK commit."""

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import TypedDict, cast

sys.dont_write_bytecode = True
from licences import collect  # noqa: E402
from validate import validate  # noqa: E402


class ComponentInput(TypedDict):
    """Declared binary and transitive source inputs from the staging rule."""

    binary: str
    inputs: list[str]


class StageSpec(TypedDict):
    """Internal staging manifest emitted by packages/build.bzl."""

    components: dict[str, ComponentInput]
    sources: list[str]
    version: str
    rustc: str
    native: bool


DESTINATIONS = {"gcsl": "usr/bin/gcsl"}


def copy(source: Path, target: Path) -> None:
    """Copy a file while normalizing its mode for an installed payload."""
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    target.chmod(0o755 if source.stat().st_mode & 0o111 else 0o644)


def split_debug(component: str, elf: Path, output: Path) -> None:
    """Split ELF debug data and record the GNU build ID and debug link."""
    notes = subprocess.check_output(["readelf", "-n", str(elf)], text=True)
    match = re.search(r"Build ID: ([0-9a-f]+)", notes)
    if not match:
        raise ValueError(f"missing build ID: {elf}")
    build_id = match.group(1)
    debug = (
        output / f"debug/{component}/usr/lib/debug/.build-id/{build_id[:2]}/{build_id[2:]}.debug"
    )
    debug.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["objcopy", "--only-keep-debug", elf, debug], check=True)
    subprocess.run(["strip", "--strip-unneeded", elf], check=True)
    subprocess.run(["objcopy", f"--add-gnu-debuglink={debug}", elf], check=True)


def source_files(inputs: list[Path]) -> list[Path]:
    """Resolve declared source files and generated bindings, rejecting missing inputs."""
    files = set()
    for path in inputs:
        if path.is_dir():
            # Generated bindings and build-script source products, not compiled archives.
            files.update(
                p for p in path.rglob("*") if p.is_file() and p.suffix in {".rs", ".h", ".c"}
            )
        elif path.is_file():
            files.add(path)
        else:
            raise ValueError(f"missing declared input: {path}")
    return sorted(files)


def stage_data(sources: list[Path], output: Path) -> None:
    """Install each declared manual in its numbered section."""
    for source in sources:
        if source.parent != Path("docs/man"):
            continue
        section = source.suffix.removeprefix(".")
        if section not in {"1", "3", "5"}:
            raise ValueError(f"unexpected manual section: {source}")
        target = output / f"usr/share/man/man{section}/{source.name}.gz"
        target.parent.mkdir(parents=True, exist_ok=True)
        with target.open("wb") as stream:
            subprocess.run(["gzip", "-n", "-9", "-c", str(source)], stdout=stream, check=True)


def check_versions(sources: list[Path], version: str) -> None:
    """Reject an omitted package manifest or a version that differs from the build rule."""
    manifest = Path("Cargo.toml")
    if manifest not in sources:
        raise ValueError("missing package manifest input")
    package = tomllib.loads(manifest.read_text(encoding="utf-8"))["workspace"]["package"]
    if package["version"] != version:
        raise ValueError("package version differs from Bazel")


def _stage_components(
    components: dict[str, ComponentInput], output: Path, debug_inputs: set[Path]
) -> dict[str, str]:
    if set(components) != set(DESTINATIONS):
        raise ValueError("incomplete or unexpected GCSL binary set")
    crate_inputs = set(debug_inputs)
    for name, component in components.items():
        elf = output / DESTINATIONS[name]
        copy(Path(component["binary"]), elf)
        split_debug("gcsl", elf, output)
        inputs = source_files([Path(path) for path in component["inputs"]])
        debug_inputs.update(inputs)
        crate_inputs.update(inputs)
    expression = collect(sorted(crate_inputs), output / "licenses")
    return {"gcsl": expression, "debugsource": expression}


def _stage_debug_sources(debug_inputs: set[Path], output: Path) -> None:
    # Only source inputs declared by Bazel enter debugsource; never walk a checkout or cache.
    debug_root = output / "usr/src/debug/dev.peios.gcsl"
    for source in sorted(debug_inputs):
        if source.suffix in {".so", ".a", ".rlib", ".o"}:
            continue
        if source.is_absolute() or ".." in source.parts:
            raise ValueError(f"non-relative declared source: {source}")
        copy(source, debug_root / source)


def _check_licences(expressions: dict[str, str]) -> None:
    for path in sorted(Path("packages/definitions").glob("*.json")):
        definition = json.loads(path.read_text(encoding="utf-8"))
        name = definition["package"]["name"]
        component = name.removeprefix("dev.peios.").removesuffix("-debuginfo")
        if component == "gcsl-debugsource":
            component = "debugsource"
        if component in expressions and definition["package"]["license"] != expressions[component]:
            raise ValueError(f"{path}: licence expression differs: {expressions[component]}")


def _write_build_record(
    spec: StageSpec, output: Path, debug_inputs: set[Path], expressions: dict[str, str]
) -> None:
    fingerprints = {
        str(path): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(debug_inputs)
        if path.is_file()
    }
    record = {
        "version": spec["version"],
        "userspace_abi": "peios-native" if spec["native"] else "debian-development",
        "release_ready": False,
        "signed": False,
        "checks": "native-hardening-and-payload" if spec["native"] else "development-payload",
        "input_digest": hashlib.sha256(
            json.dumps(fingerprints, sort_keys=True).encode("utf-8")
        ).hexdigest(),
        "licences": expressions,
    }
    (output / "build.json").write_text(
        json.dumps(record, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )


def main() -> None:
    """Stage the declared build inputs and write provenance after validation succeeds."""
    spec = cast(StageSpec, json.loads(Path(sys.argv[1]).read_text(encoding="utf-8")))
    output = Path(sys.argv[2])
    output.mkdir(parents=True, exist_ok=True)
    sources = [Path(path) for path in spec["sources"]]
    check_versions(sources, spec["version"])
    debug_inputs = set(sources)
    expressions = _stage_components(spec["components"], output, debug_inputs)
    _stage_debug_sources(debug_inputs, output)
    _check_licences(expressions)
    stage_data(sources, output)
    os.environ["RUSTC"] = str(Path(spec["rustc"]).absolute())
    validate(output, native=spec["native"])
    _write_build_record(spec, output, debug_inputs, expressions)


if __name__ == "__main__":
    main()
