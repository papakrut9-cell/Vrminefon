#!/usr/bin/env bash
# Downloads Mojang's pinned game files (version JSON, client and server jars, the client
# libraries) for a build-time exporter compile, exactly the files the app's first-run setup
# (android_setup.rs) downloads on the phone. Every file's SHA-1 is checked against
# research-lock.json or the version JSON. The files pass through the build and are never
# published, committed or shipped: Horizon never ships Minecraft's files.
# Usage: fetch_game_files.sh [OUT_DIR]
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT="${1:-game-files}"
python3 - "$OUT" <<'PY'
import hashlib, json, pathlib, sys, urllib.request

out = pathlib.Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
lock = json.loads(pathlib.Path("research-lock.json").read_text())
game = lock["minecraft"]
version = game["version"]


def check(name: str, data: bytes, sha1: str) -> None:
    got = hashlib.sha1(data).hexdigest()
    if got != sha1:
        raise SystemExit(f"{name}: SHA-1 {got}, expected {sha1}")


def get(url: str, name: str, sha1: str | None = None) -> bytes:
    print(f"GET {url}")
    data = urllib.request.urlopen(url, timeout=120).read()
    if sha1 is not None:
        check(name, data, sha1)
    return data


version_json = get(
    f"https://piston-meta.mojang.com/v1/packages/{game['manifest_sha1']}/{version}.json",
    "version JSON",
    game["manifest_sha1"],
)
(out / "version.json").write_bytes(version_json)
manifest = json.loads(version_json)

client = manifest["downloads"]["client"]["url"]
client_sha1 = manifest["downloads"]["client"]["sha1"]
if client_sha1 != game["client_sha1"]:
    raise SystemExit(
        f"Mojang's client jar moved ({client_sha1}, pinned {game['client_sha1']}); update research-lock.json"
    )
(out / "client.jar").write_bytes(get(client, "client.jar", game["client_sha1"]))

(out / "server.jar").write_bytes(
    get(
        f"https://piston-data.mojang.com/v1/objects/{game['server_sha1']}/server.jar",
        "server.jar",
        game["server_sha1"],
    )
)

libraries = out / "libraries"
n = 0
for library in manifest.get("libraries", []):
    artifact = library.get("downloads", {}).get("artifact", {})
    path, url, sha1 = artifact.get("path"), artifact.get("url"), artifact.get("sha1")
    if not (path and url and sha1):
        continue
    target = libraries / path
    if target.is_file() and hashlib.sha1(target.read_bytes()).hexdigest() == sha1:
        continue
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(get(url, path, sha1))
    n += 1
print(f"{n} libraries")
PY
echo "Mojang's pinned files are in $OUT"
