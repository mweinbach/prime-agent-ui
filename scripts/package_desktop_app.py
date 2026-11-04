#!/usr/bin/env python3
"""Assemble and sign `dist/Prime Agent.app` for macOS.

Follows strict macOS bundle sealing order:
1. Binaries (`prime-agent-ui`, `prime-agent`) placed under `Contents/MacOS/`.
2. Resources (`AppIcon.icns`, `prime-agent-runtime/`, `skills/`) placed under
   `Contents/Resources/` BEFORE signing; no root symlinks, no user credentials.
3. Ad-hoc signed with `codesign --force --deep --sign -` and verified with
   `codesign --verify --deep --strict`.
"""

from __future__ import annotations

import argparse
import math
import plistlib
import shutil
import struct
import subprocess
import tempfile
import zlib
from pathlib import Path


def _png_chunk(tag: bytes, data: bytes) -> bytes:
    crc = zlib.crc32(tag + data) & 0xFFFFFFFF
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", crc)


def generate_icon_png(size: int = 1024) -> bytes:
    """Render a warm Obsidian-Clay + Terracotta squircle icon as 8-bit RGBA PNG."""
    rows: list[bytes] = []
    half = size / 2.0
    radius = size * 0.435

    for y in range(size):
        row = bytearray([0])  # filter type 0
        ny = (y + 0.5 - half) / radius
        for x in range(size):
            nx = (x + 0.5 - half) / radius
            # Continuous superellipse (macOS squircle, n=4.6)
            d = (abs(nx) ** 4.6 + abs(ny) ** 4.6) ** (1.0 / 4.6)
            if d > 1.015:
                row.extend((0, 0, 0, 0))
                continue

            edge_alpha = max(0.0, min(1.0, (1.015 - d) / 0.02))

            # Warm dark espresso-stone gradient with soft terracotta radial glow
            grad = 0.5 * (ny + 1.0)
            r = 34.0 - 10.0 * grad
            g = 32.0 - 10.0 * grad
            b = 28.0 - 9.0 * grad

            glow_d = math.hypot(nx + 0.15, ny + 0.22)
            glow = max(0.0, 1.0 - glow_d / 1.15) ** 2.2
            r += 58.0 * glow
            g += 24.0 * glow
            b += 14.0 * glow

            # Subtle inner rim highlight near squircle border
            if 0.955 <= d <= 0.995:
                rim = 1.0 - abs(d - 0.975) / 0.02
                r += 28.0 * rim
                g += 24.0 * rim
                b += 20.0 * rim

            # Crisp central 4-point editorial star / RLM node glyph
            gx = abs(nx)
            gy = abs(ny)
            star_metric = (gx**0.62 + gy**0.62) ** (1.0 / 0.62)
            ring_d = abs(math.hypot(nx, ny) - 0.52)

            if star_metric < 0.36:
                t = max(0.0, min(1.0, (0.36 - star_metric) / 0.025))
                # Warm terracotta-ivory core
                core = max(0.0, 1.0 - math.hypot(nx, ny) / 0.30)
                sr = 217.0 + 32.0 * core
                sg = 119.0 + 108.0 * core
                sb = 87.0 + 140.0 * core
                r = r * (1.0 - t) + sr * t
                g = g * (1.0 - t) + sg * t
                b = b * (1.0 - t) + sb * t
            elif ring_d < 0.028:
                t = max(0.0, min(1.0, (0.028 - ring_d) / 0.012)) * 0.68
                r = r * (1.0 - t) + 217.0 * t
                g = g * (1.0 - t) + 119.0 * t
                b = b * (1.0 - t) + 87.0 * t

            # Three subtle RLM child satellite nodes
            for sx, sy in ((-0.52, 0.0), (0.26, -0.45), (0.26, 0.45)):
                nd = math.hypot(nx - sx, ny - sy)
                if nd < 0.078:
                    nt = max(0.0, min(1.0, (0.078 - nd) / 0.018))
                    r = r * (1.0 - nt) + 242.0 * nt
                    g = g * (1.0 - nt) + 239.0 * nt
                    b = b * (1.0 - nt) + 233.0 * nt

            row.extend(
                (
                    int(max(0, min(255, round(r)))),
                    int(max(0, min(255, round(g)))),
                    int(max(0, min(255, round(b)))),
                    int(max(0, min(255, round(edge_alpha * 255.0)))),
                )
            )
        rows.append(bytes(row))

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    compressed = zlib.compress(b"".join(rows), level=6)
    return (
        b"\x89PNG\r\n\x1a\n"
        + _png_chunk(b"IHDR", ihdr)
        + _png_chunk(b"IDAT", compressed)
        + _png_chunk(b"IEND", b"")
    )


def build_icns(out_icns: Path) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        master_png = tmp_path / "master.png"
        master_png.write_bytes(generate_icon_png(512))

        iconset = tmp_path / "AppIcon.iconset"
        iconset.mkdir()

        specs = [
            (16, "icon_16x16.png"),
            (32, "icon_16x16@2x.png"),
            (32, "icon_32x32.png"),
            (64, "icon_32x32@2x.png"),
            (128, "icon_128x128.png"),
            (256, "icon_128x128@2x.png"),
            (256, "icon_256x256.png"),
            (512, "icon_256x256@2x.png"),
            (512, "icon_512x512.png"),
            (512, "icon_512x512@2x.png"),
        ]
        for px_size, name in specs:
            dest = iconset / name
            subprocess.run(
                ["sips", "-z", str(px_size), str(px_size), str(master_png), "--out", str(dest)],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )

        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(out_icns)],
            check=True,
        )


def parse_workspace_version(repo_root: Path) -> str:
    for line in (repo_root / "Cargo.toml").read_text().splitlines():
        line = line.strip()
        if line.startswith('version = "'):
            return line.split('"')[1]
    return "0.10.2"


def main() -> None:
    parser = argparse.ArgumentParser(description="Package Prime Agent.app")
    parser.add_argument("--profile", choices=["debug", "release"], default="release")
    parser.add_argument("--out-dir", default="dist")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parent.parent
    target_dir = repo_root / "target" / args.profile
    ui_bin = target_dir / "prime-agent-ui"
    cli_bin = target_dir / "prime-agent"

    if not ui_bin.is_file() or not cli_bin.is_file():
        raise SystemExit(
            f"Missing built binaries in {target_dir}; build `pa-desktop` and `pa-cli` first."
        )

    out_root = repo_root / args.out_dir
    app_dir = out_root / "Prime Agent.app"
    if app_dir.exists():
        shutil.rmtree(app_dir)

    contents = app_dir / "Contents"
    macos_dir = contents / "MacOS"
    resources_dir = contents / "Resources"
    macos_dir.mkdir(parents=True)
    resources_dir.mkdir(parents=True)

    # 1. Copy executables
    shutil.copy2(ui_bin, macos_dir / "prime-agent-ui")
    shutil.copy2(cli_bin, macos_dir / "prime-agent")
    (macos_dir / "prime-agent-ui").chmod(0o755)
    (macos_dir / "prime-agent").chmod(0o755)

    # 2. Assemble ALL resources under Contents/Resources BEFORE signing
    build_icns(resources_dir / "AppIcon.icns")

    ignore_caches = shutil.ignore_patterns("__pycache__", "*.pyc", ".pytest_cache", ".venv", ".DS_Store")
    shutil.copytree(
        repo_root / "prime-agent-runtime",
        resources_dir / "prime-agent-runtime",
        ignore=ignore_caches,
    )
    shutil.copytree(
        repo_root / "skills",
        resources_dir / "skills",
        ignore=ignore_caches,
    )

    version = parse_workspace_version(repo_root)
    info = {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": "Prime Agent",
        "CFBundleExecutable": "prime-agent-ui",
        "CFBundleIconFile": "AppIcon",
        "CFBundleIdentifier": "ai.primeintellect.prime-agent.desktop",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": "Prime Agent",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "LSMinimumSystemVersion": "13.0",
        "NSHighResolutionCapable": True,
        "NSSupportsAutomaticGraphicsSwitching": True,
    }
    with (contents / "Info.plist").open("wb") as fp:
        plistlib.dump(info, fp)

    # 3. Sign after resource assembly and run strict verification
    subprocess.run(
        ["codesign", "--force", "--deep", "--sign", "-", str(app_dir)],
        check=True,
    )
    subprocess.run(
        ["codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app_dir)],
        check=True,
    )
    print(f"Packaged and verified: {app_dir}")


if __name__ == "__main__":
    main()
