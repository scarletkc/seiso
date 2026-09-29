import base64
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import threading
import unittest
from urllib.error import HTTPError
from urllib.parse import quote
from urllib.request import urlopen

from scripts.release import npm_registry


class RegistryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def archive(self, filename, **manifest):
        with tarfile.open(self.root / filename, "w:gz") as archive:
            data = json.dumps(manifest).encode()
            member = tarfile.TarInfo("package/package.json")
            member.size = len(data)
            archive.addfile(member, io.BytesIO(data))
        return (self.root / filename).read_bytes()

    def serve(self):
        registry = npm_registry.Registry(self.root)
        self.addCleanup(registry.server_close)
        thread = threading.Thread(target=registry.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(registry.shutdown)
        return registry

    def get(self, url):
        with urlopen(url, timeout=10) as response:
            return response.read()

    def test_serves_scoped_packuments_with_platform_fields_and_verifiable_tarballs(self):
        data = self.archive("scarletkc-seiso-linux-x64-musl-1.2.3.tgz", name="@scarletkc/seiso-linux-x64-musl",
                            version="1.2.3", os=["linux"], cpu=["x64"], libc=["musl"])
        self.archive("scarletkc-seiso-1.2.3.tgz", name="@scarletkc/seiso", version="1.2.3",
                     optionalDependencies={"@scarletkc/seiso-linux-x64-musl": "1.2.3"})
        registry = self.serve()
        packument = json.loads(self.get(f"{registry.url}/{quote('@scarletkc/seiso-linux-x64-musl', safe='@')}"))
        self.assertEqual(packument["dist-tags"], {"latest": "1.2.3"})
        manifest = packument["versions"]["1.2.3"]
        self.assertEqual((manifest["os"], manifest["cpu"], manifest["libc"]), (["linux"], ["x64"], ["musl"]))
        self.assertEqual(manifest["_id"], "@scarletkc/seiso-linux-x64-musl@1.2.3")
        self.assertEqual(self.get(manifest["dist"]["tarball"]), data)
        self.assertEqual(manifest["dist"]["integrity"],
                         "sha512-" + base64.b64encode(hashlib.sha512(data).digest()).decode())
        self.assertEqual(manifest["dist"]["shasum"], hashlib.sha1(data).hexdigest())
        main = json.loads(self.get(f"{registry.url}/@scarletkc%2fseiso"))
        self.assertEqual(main["versions"]["1.2.3"]["optionalDependencies"],
                         {"@scarletkc/seiso-linux-x64-musl": "1.2.3"})

    def test_unknown_packages_and_tarballs_are_absent(self):
        self.archive("seiso.tgz", name="@scarletkc/seiso", version="1.2.3")
        registry = self.serve()
        for path in ["/@scarletkc%2fseiso-linux-ia32", "/-/missing.tgz", "/npm"]:
            with self.subTest(path=path), self.assertRaises(HTTPError) as error:
                self.get(registry.url + path)
            self.assertEqual(error.exception.code, 404)
            error.exception.close()

    def test_empty_or_duplicate_archives_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "No npm archives"):
            npm_registry.load(self.root, "http://127.0.0.1")
        self.archive("a.tgz", name="@scarletkc/seiso", version="1.2.3")
        self.archive("b.tgz", name="@scarletkc/seiso", version="1.2.3")
        with self.assertRaisesRegex(ValueError, "More than one"):
            npm_registry.load(self.root, "http://127.0.0.1")

    def test_command_runs_against_the_registry_and_keeps_its_exit_status(self):
        self.archive("seiso.tgz", name="@scarletkc/seiso", version="1.2.3")
        script = ("import json, os, sys, urllib.request; "
                  "url = os.environ['npm_config_registry'] + '@scarletkc%2fseiso'; "
                  "sys.exit(0 if json.load(urllib.request.urlopen(url))['name'] == '@scarletkc/seiso' else 1)")
        self.assertEqual(npm_registry.serve(self.root, [sys.executable, "-c", script]), 0)
        self.assertEqual(npm_registry.serve(self.root, [sys.executable, "-c", "raise SystemExit(3)"]), 3)


if __name__ == "__main__":
    unittest.main()
