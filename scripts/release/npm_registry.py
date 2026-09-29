"""Serve packed npm archives as a read-only registry while a command runs.

Usage: python -m scripts.release.npm_registry DIRECTORY -- COMMAND...
The command runs with npm_config_registry pointing at the archives in DIRECTORY,
so npm resolves optional platform packages exactly as it would from npmjs.com.
"""

import argparse
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import threading
from urllib.parse import unquote, urlsplit


def load(directory, base):
    """Return the packuments and tarballs that describe every archive in directory."""
    packuments, tarballs = {}, {}
    for archive in sorted(directory.glob("*.tgz")):
        data = archive.read_bytes()
        with tarfile.open(archive) as bundle:
            manifest = json.load(bundle.extractfile("package/package.json"))
        name, version = manifest["name"], manifest["version"]
        packument = packuments.setdefault(name, {"name": name, "dist-tags": {}, "versions": {}})
        if version in packument["versions"]:
            raise ValueError(f"More than one npm archive supplies {name} {version}")
        manifest["_id"] = f"{name}@{version}"
        manifest["dist"] = {
            "tarball": f"{base}/-/{archive.name}",
            "integrity": "sha512-" + base64.b64encode(hashlib.sha512(data).digest()).decode(),
            "shasum": hashlib.sha1(data).hexdigest(),
        }
        packument["versions"][version] = manifest
        packument["dist-tags"]["latest"] = version
        tarballs[archive.name] = data
    if not packuments:
        raise ValueError(f"No npm archives found in {directory}")
    return packuments, tarballs


class Registry(ThreadingHTTPServer):
    def __init__(self, directory):
        super().__init__(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server_port}"
        self.packuments, self.tarballs = load(directory, self.url)


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        # npm requests scoped packages as /@scope%2fname.
        path = unquote(urlsplit(self.path).path).lstrip("/")
        if path.startswith("-/"):
            body, content_type = self.server.tarballs.get(path[2:]), "application/octet-stream"
        else:
            packument = self.server.packuments.get(path)
            body = json.dumps(packument).encode() if packument else None
            content_type = "application/json"
        if body is None:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def serve(directory, command):
    registry = Registry(directory)
    threading.Thread(target=registry.serve_forever, daemon=True).start()
    print(f"Serving {', '.join(sorted(registry.packuments))} at {registry.url}", flush=True)
    environment = {**os.environ, "npm_config_registry": f"{registry.url}/"}
    try:
        return subprocess.run([shutil.which(command[0]) or command[0], *command[1:]], env=environment).returncode
    finally:
        registry.shutdown()
        registry.server_close()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("directory", type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER, help="Command after --")
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a command is required after --")
    sys.exit(serve(args.directory, command))


if __name__ == "__main__":
    main()
