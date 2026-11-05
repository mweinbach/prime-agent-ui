import base64
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from xml.etree import ElementTree as ET
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from sparkle_appcast import ARCHIVE, NAMESPACE, SPARKLE_FEED_URL, feed, publishable_feed


class AppcastTests(unittest.TestCase):
    def test_enclosure_uses_versioned_download_and_minimum_os(self):
        item = feed("0.10.3", "13.0", 124, "sig").find("channel/item")
        self.assertEqual(item.findtext(f"{{{NAMESPACE}}}version"), "0.10.3")
        self.assertEqual(item.findtext(f"{{{NAMESPACE}}}minimumSystemVersion"), "13.0")
        enclosure = item.find("enclosure")
        self.assertEqual(enclosure.get("length"), "124")
        self.assertEqual(enclosure.get("url"), "https://github.com/mweinbach/prime-agent-ui/releases/download/desktop-v0.10.3/" + ARCHIVE)

    @unittest.skipUnless(sys.platform == "darwin" and os.environ.get("SPARKLE_SDK"), "requires native Sparkle tools")
    def test_signatures_cover_final_bytes_and_match_the_app_key(self):
        # RFC 8032 test vector 1. This public test identity is used only in a temporary fixture.
        seed = base64.b64encode(bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")).decode()
        public = base64.b64encode(bytes.fromhex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")).decode()
        info = dict(SUFeedURL=SPARKLE_FEED_URL, SUPublicEDKey=public, SURequireSignedFeed=True,
                    CFBundleVersion="0.10.3", CFBundleShortVersionString="0.10.3", LSMinimumSystemVersion="13.0")
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            archive = directory / ARCHIVE
            with zipfile.ZipFile(archive, "w") as package:
                package.writestr("Prime Agent.app/Contents/Info.plist", plistlib.dumps(info))
            with patch.dict(os.environ, SPARKLE_PRIVATE_ED_KEY=seed):
                xml = publishable_feed(directory, "desktop-v0.10.3")
            enclosure = ET.parse(xml).find("channel/item/enclosure")
            self.assertEqual(int(enclosure.get("length")), archive.stat().st_size)
            signature = enclosure.get(f"{{{NAMESPACE}}}edSignature")
            verify = [Path(os.environ["SPARKLE_SDK"]) / "bin/sign_update", "--ed-key-file", "-", "--verify"]
            subprocess.run([*verify, archive, signature], input=seed, text=True, check=True)
            xml.write_bytes(xml.read_bytes().replace(b"Stable updates", b"Edited updates"))
            self.assertNotEqual(subprocess.run([*verify, xml], input=seed, text=True).returncode, 0)
            with self.assertRaises(ValueError):
                publishable_feed(directory, "desktop-v0.10.4")
            with patch.dict(os.environ, SPARKLE_PRIVATE_ED_KEY=base64.b64encode(os.urandom(32)).decode()):
                with self.assertRaises(subprocess.CalledProcessError):
                    publishable_feed(directory, "desktop-v0.10.3")
