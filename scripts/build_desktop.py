#!/usr/bin/env python3
"""Build and verify a native Prime Agent Desktop release archive."""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

from package_desktop_app import (
    DESKTOP_NOTICES,
    HARNESS_NOTICES,
    package_app,
    parse_workspace_version,
    stage_binaries,
    stage_resources,
    verify_app,
)


REPO_ROOT = Path(__file__).resolve().parent.parent
NATIVE_TARGETS = {
    ("macos", "arm64"): "aarch64-apple-darwin",
    ("linux", "x86_64"): "x86_64-unknown-linux-gnu",
    ("windows", "arm64"): "aarch64-pc-windows-msvc",
    ("windows", "x86_64"): "x86_64-pc-windows-msvc",
}


def native_target(requested_arch: str | None) -> tuple[str, str, str]:
    system = {"darwin": "macos", "win32": "windows"}.get(sys.platform, sys.platform)
    machine = platform.machine().lower()
    arch = {"aarch64": "arm64", "amd64": "x86_64"}.get(machine, machine)
    target = NATIVE_TARGETS.get((system, arch))
    if target is None:
        raise ValueError(
            f"Unsupported host {system}/{machine}. Use native macOS arm64, Linux x86_64, "
            "or Windows arm64/x86_64."
        )
    if requested_arch is not None and requested_arch != arch:
        raise ValueError(
            f"Requested {requested_arch} on {system}/{arch}; cross-builds are not supported."
        )

    rustc_version = subprocess.check_output(["rustc", "-vV"], text=True, cwd=REPO_ROOT)
    fields = dict(line.split(": ", 1) for line in rustc_version.splitlines() if ": " in line)
    host = fields.get("host", "unknown")
    if host != target:
        raise ValueError(f"Rust host is {host}; install and activate the native {target} toolchain.")
    return system, arch, target


def git_output(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()


def pinned_harness() -> str:
    harness = REPO_ROOT / "harness"
    if not (harness / "Cargo.toml").is_file() or not (harness / ".git").exists():
        raise ValueError("Harness is not initialized. Run `git submodule update --init --recursive`.")
    expected = git_output(REPO_ROOT, "rev-parse", "HEAD:harness")
    current = git_output(harness, "rev-parse", "HEAD")
    if current != expected:
        raise ValueError(
            f"Harness is at {current}; this checkout pins {expected}. Use the pinned commit."
        )
    if git_output(harness, "status", "--porcelain", "--untracked-files=no"):
        raise ValueError("Harness has tracked changes. Build from a clean checkout of the pin.")
    return expected


def build_binary(manifest: Path, package: str, binary: str, windows_target: str | None) -> Path:
    command = [
        "cargo", "build", "--release", "--locked", "--jobs", "6",
        "--target-dir", str(REPO_ROOT / "target"), "--manifest-path", str(manifest),
        "--package", package, "--bin", binary, "--message-format=json-render-diagnostics",
    ]
    env = os.environ.copy()
    if windows_target:
        # Explicit target keeps the static MSVC runtime flags off host proc macros and build scripts.
        command += ["--target", windows_target]
        if "CARGO_ENCODED_RUSTFLAGS" in env:
            flags = env["CARGO_ENCODED_RUSTFLAGS"]
            env["CARGO_ENCODED_RUSTFLAGS"] = f"{flags}\x1f" if flags else ""
            env["CARGO_ENCODED_RUSTFLAGS"] += "-C\x1ftarget-feature=+crt-static"
        else:
            env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + " -C target-feature=+crt-static").strip()

    result = subprocess.run(
        command, cwd=REPO_ROOT, env=env, stdout=subprocess.PIPE, encoding="utf-8", check=True
    )
    # Use the artifact from this Cargo invocation, not a possibly stale file in target/release.
    for line in result.stdout.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if (
            message.get("reason") == "compiler-artifact"
            and message["target"]["name"] == binary
            and "bin" in message["target"]["kind"]
            and message["executable"]
        ):
            return Path(message["executable"])
    raise ValueError(f"Cargo did not report a built {binary} executable.")


def verify_architecture(binary: Path, system: str, arch: str) -> None:
    if system == "macos":
        reported = subprocess.check_output(["lipo", "-archs", str(binary)], text=True).strip()
        if reported == arch:
            return
    else:
        with binary.open("rb") as exe:
            header = exe.read(64)
            # Little-endian ELF64 with e_machine = EM_X86_64.
            if (
                system == "linux" and header.startswith(b"\x7fELF\x02\x01")
                and header[18:20] == b"\x3e\0"
            ):
                return
            if system == "windows" and len(header) == 64 and header.startswith(b"MZ"):
                # DOS e_lfanew points to the PE header.
                exe.seek(int.from_bytes(header[60:64], "little"))
                machine = {"arm64": 0xAA64, "x86_64": 0x8664}[arch]
                if exe.read(6) == b"PE\0\0" + machine.to_bytes(2, "little"):
                    return
    raise ValueError(f"{binary.name} is not a native {system}/{arch} executable.")


def verify_package(root: Path, bin_dir: Path, resources: Path, system: str, arch: str) -> None:
    required_resources = [
        "prime-agent-runtime/pyproject.toml",
        "prime-agent-runtime/src/rlm/__init__.py",
        "prime-agent-runtime/src/rlm/repl.py",
        "skills/agent-message/SKILL.md",
        *(f"licenses/harness/{path}" for path in HARNESS_NOTICES.values()),
        *(f"licenses/desktop/{Path(path).name}" for path in DESKTOP_NOTICES),
    ]
    for path in required_resources:
        if not (resources / path).is_file():
            raise ValueError(f"Missing packaged resource: {path}")

    # The version probes do not start a window or daemon. Keep credentials and shared profiles out.
    home = root / "test-home"
    home.mkdir()
    env = {key: str(home) for key in ("HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP")}
    if system == "windows":
        env["SYSTEMROOT"] = os.environ["SYSTEMROOT"]
    env.update(DO_NOT_TRACK="1", PI_OFFLINE="1", PI_PACKAGE_DIR=str(resources))
    suffix = ".exe" if system == "windows" else ""
    for name, expected in [
        ("prime-agent-ui", f"prime-agent-ui {parse_workspace_version(REPO_ROOT)}"),
        ("prime-agent", parse_workspace_version(REPO_ROOT / "harness")),
    ]:
        binary = bin_dir / f"{name}{suffix}"
        verify_architecture(binary, system, arch)
        version = subprocess.check_output(
            [str(binary), "--version"], cwd=home, env=env, stdin=subprocess.DEVNULL,
            text=True, timeout=20,
        ).strip()
        if version != expected:
            raise ValueError(f"{binary.name} reports {version!r}; expected {expected!r}.")


def normalize_tar_owner(entry: tarfile.TarInfo) -> tarfile.TarInfo:
    entry.uid = entry.gid = 0
    entry.uname = entry.gname = "root"
    return entry


def release_archive(
    out_dir: Path, binaries: tuple[Path, Path], harness_ref: str, system: str, arch: str
) -> Path:
    stem = f"prime-agent-ui-{system}-{arch}"
    out_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".build-", dir=out_dir) as tmp:
        work = Path(tmp)
        unpacked = work / "unpacked"
        unpacked.mkdir()
        if system == "macos":
            app = package_app(REPO_ROOT, work, binaries, harness_ref)
            archive = work / f"{stem}.zip"
            subprocess.run(
                ["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app), str(archive)],
                check=True,
            )
            subprocess.run(["ditto", "-x", "-k", str(archive), str(unpacked)], check=True)
            unpacked_app = unpacked / app.name
            verify_app(unpacked_app)
            contents = unpacked_app / "Contents"
            verify_package(work, contents / "MacOS", contents / "Resources", system, arch)
        else:
            stage = work / stem
            stage_binaries(binaries, stage)
            stage_resources(REPO_ROOT, stage / "resources", harness_ref)
            if system == "windows":
                archive = Path(shutil.make_archive(str(work / stem), "zip", work, stem))
            else:
                archive = work / f"{stem}.tar.gz"
                with tarfile.open(archive, "w:gz") as tar:
                    tar.add(stage, arcname=stem, filter=normalize_tar_owner)
            shutil.unpack_archive(archive, unpacked)
            verify_package(work, unpacked / stem, unpacked / stem / "resources", system, arch)
        destination = out_dir / archive.name
        archive.replace(destination)
        return destination


def main() -> None:
    if sys.version_info < (3, 10):
        raise SystemExit("Building Prime Agent Desktop requires Python 3.10 or later.")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--arch", choices=("arm64", "x86_64"), help="assert the native build architecture"
    )
    parser.add_argument(
        "--out-dir", type=Path, default=REPO_ROOT / "dist",
        help="archive directory (default: dist in the checkout)",
    )
    args = parser.parse_args()

    try:
        system, arch, target = native_target(args.arch)
        harness_ref = pinned_harness()
        print(f"Building {target} with harness {harness_ref}", flush=True)
        windows_target = target if system == "windows" else None
        cli = build_binary(REPO_ROOT / "harness/Cargo.toml", "pa-cli", "prime-agent", windows_target)
        ui = build_binary(REPO_ROOT / "Cargo.toml", "pa-desktop", "prime-agent-ui", windows_target)
        archive = release_archive(args.out_dir.resolve(), (ui, cli), harness_ref, system, arch)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f"Desktop build failed: {error}") from None
    print(f"Packaged and verified: {archive}")


if __name__ == "__main__":
    main()
