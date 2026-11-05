#!/usr/bin/env python3
"""Sign the final, already packaged stable macOS ZIP and its single-item Sparkle feed."""
import argparse
import base64
import os
from pathlib import Path
import plistlib
import re
import subprocess
import tempfile
import xml.etree.ElementTree as ET
import zipfile

from package_desktop_app import SPARKLE_FEED_URL

NAMESPACE = "http://www.andymatuschak.org/xml-namespaces/sparkle"
RELEASES = "https://github.com/mweinbach/prime-agent-ui/releases"
ARCHIVE = "prime-agent-ui-macos-arm64.zip"


def feed(version: str, minimum_os: str, length: int, signature: str) -> ET.ElementTree:
    ET.register_namespace("sparkle", NAMESPACE)
    rss = ET.Element("rss", version="2.0")
    channel = ET.SubElement(rss, "channel")
    ET.SubElement(channel, "title").text = "Prime Agent Desktop for macOS"
    ET.SubElement(channel, "link").text = RELEASES
    ET.SubElement(channel, "description").text = "Stable updates for Apple Silicon Macs."
    item = ET.SubElement(channel, "item")
    ET.SubElement(item, "title").text = f"Prime Agent Desktop {version}"
    ET.SubElement(item, "link").text = f"{RELEASES}/tag/desktop-v{version}"
    for name, value in (("version", version), ("shortVersionString", version), ("minimumSystemVersion", minimum_os)):
        ET.SubElement(item, ET.QName(NAMESPACE, name)).text = value
    enclosure = ET.SubElement(item, "enclosure", url=f"{RELEASES}/download/desktop-v{version}/{ARCHIVE}",
                              length=str(length), type="application/octet-stream")
    enclosure.set(ET.QName(NAMESPACE, "edSignature"), signature)
    ET.indent(rss)
    return ET.ElementTree(rss)


def publishable_feed(directory: Path, tag: str) -> Path:
    if not re.fullmatch(r"desktop-v[0-9]+\.[0-9]+\.[0-9]+", tag):
        raise ValueError(f"Expected a stable desktop tag, got {tag!r}.")
    version = tag.removeprefix("desktop-v")
    archive = directory / ARCHIVE
    with zipfile.ZipFile(archive) as package:
        info = plistlib.loads(package.read("Prime Agent.app/Contents/Info.plist"))
    public_key = info.get("SUPublicEDKey", "")
    if (info.get("SUFeedURL") != SPARKLE_FEED_URL or info.get("CFBundleVersion") != version
            or info.get("CFBundleShortVersionString") != version or not info.get("SURequireSignedFeed")
            or len(base64.b64decode(public_key, validate=True)) != 32):
        raise ValueError("The packaged updater settings or version do not match this release.")

    # Hand the secret only to the official signing tool over stdin.
    private_key = os.environ.pop("SPARKLE_PRIVATE_ED_KEY", "").strip()
    if not private_key:
        raise ValueError("Set the SPARKLE_PRIVATE_ED_KEY repository secret.")
    signer = Path(os.environ["SPARKLE_SDK"]) / "bin/sign_update"

    def sign(*args):
        return subprocess.run([signer, "--ed-key-file", "-", *args], check=True, input=private_key,
                              text=True, stdout=subprocess.PIPE).stdout.strip()

    signature = sign("-p", archive)
    if len(base64.b64decode(signature, validate=True)) != 64:
        raise ValueError("Invalid archive signature returned by sign_update.")
    # Verify against the PUBLIC key inside the actual shipped app, independently of the signer.
    subprocess.run(["xcrun", "swift", Path(__file__).with_name("verify_sparkle.swift"),
                    public_key, signature, archive], check=True)
    with tempfile.TemporaryDirectory(dir=directory) as temp:
        xml = Path(temp) / "appcast.xml"
        feed(version, info["LSMinimumSystemVersion"], archive.stat().st_size, signature).write(
            xml, encoding="utf-8", xml_declaration=True)
        sign(xml)
        sign("--verify", xml)
        return xml.replace(directory / "appcast.xml")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("tag")
    args = parser.parse_args()
    print(publishable_feed(args.directory.resolve(), args.tag))
