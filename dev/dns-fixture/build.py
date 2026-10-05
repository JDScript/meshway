#!/usr/bin/env python3
"""Explicit parent-run OS image build. No Rust build, ambient apt repository or live service.

The locked complete package-index hashes pin every dependency version, filename,
and artifact digest. The expanded exact closure is saved before downloading it.
"""
import argparse
import hashlib
import json
import lzma
from pathlib import Path
import re
import subprocess
import urllib.request

HERE = Path(__file__).resolve().parent


def digest(data):
    return hashlib.sha256(data).hexdigest()


def fetch(url, expected):
    with urllib.request.urlopen(url, timeout=120) as response:
        data = response.read()
    if digest(data) != expected:
        raise RuntimeError("Pinned OS asset digest mismatch: " + url)
    return data


def records(text):
    result = {}
    for paragraph in text.split("\n\n"):
        fields = {}
        for line in paragraph.splitlines():
            if line and not line[0].isspace():
                key, value = line.split(":", 1)
                fields[key] = value.strip()
        if "Package" in fields:
            result[fields["Package"]] = fields
    return result


def closure(index, roots):
    providers = {}
    for name, record in sorted(index.items()):
        for provided in record.get("Provides", "").split(","):
            if provided.strip():
                providers.setdefault(provided.strip().split()[0], name)
    selected = {}
    pending = list(roots)
    while pending:
        name = pending.pop()
        if name in selected:
            continue
        record = index[name]
        selected[name] = record
        for field in ("Pre-Depends", "Depends"):
            for group in record.get(field, "").split(","):
                if not group.strip():
                    continue
                alternatives = [re.split(r"[\s:(]", item.strip(), maxsplit=1)[0]
                                for item in group.split("|")]
                candidate = next((item if item in index else providers[item]
                                  for item in alternatives if item in index or item in providers), None)
                if candidate is None:
                    raise RuntimeError("Unresolved locked package dependency: " + group)
                pending.append(candidate)
    return [selected[name] for name in sorted(selected)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=("linux/amd64", "linux/arm64"), required=True)
    parser.add_argument("--output", type=Path, required=True, help="new private build/support directory")
    parser.add_argument("--resume", action="store_true", help="resume this exact locked build's verified downloads")
    args = parser.parse_args()
    if not args.output.is_absolute():
        raise RuntimeError("OS build output must be an absolute private directory.")
    args.output.mkdir(parents=True, exist_ok=args.resume, mode=0o700)
    lock_bytes = (HERE / "assets.lock.json").read_bytes()
    lock = json.loads(lock_bytes)
    arch = args.platform.split("/")[1]
    fetch(lock["snapshot"] + "/dists/trixie/Release", lock["releaseSha256"])
    index_bytes = fetch(lock["snapshot"] + f"/dists/trixie/main/binary-{arch}/Packages.xz", lock["indexes"][arch])
    selected = closure(records(lzma.decompress(index_bytes).decode()), lock["packages"])
    packages = [{"name": item["Package"], "version": item["Version"], "architecture": item["Architecture"],
                 "filename": item["Filename"], "sha256": item["SHA256"]} for item in selected]
    record = {"base": lock["base"], "platform": args.platform, "lockSha256": digest(lock_bytes),
              "packages": packages, "status": "candidate"}
    manifest = args.output / "image-manifest.json"
    if args.resume:
        previous = json.loads(manifest.read_text())
        if any(previous.get(key) != record[key] for key in ("base", "platform", "lockSha256", "packages")):
            raise RuntimeError("Existing OS build belongs to different locked inputs.")
    manifest.write_text(json.dumps(record, indent=2) + "\n")
    debs = args.output / "debs"
    debs.mkdir(exist_ok=args.resume)
    for item in packages:
        path = debs / Path(item["filename"]).name
        if path.exists():
            if digest(path.read_bytes()) != item["sha256"]:
                raise RuntimeError("Existing OS package digest mismatch: " + path.name)
        else:
            path.write_bytes(fetch(lock["snapshot"] + "/" + item["filename"], item["sha256"]))
    dockerfile = """FROM {base}
COPY debs/ /tmp/debs/
RUN printf '#!/bin/sh\\nexit 101\\n' >/usr/sbin/policy-rc.d && chmod 755 /usr/sbin/policy-rc.d && \\
    DEBIAN_FRONTEND=noninteractive apt-get --no-remove --no-install-recommends -y install /tmp/debs/*.deb && \\
    dpkg-query -W -f='${{binary:Package}}\\t${{Version}}\\n' >/usr/share/meshway-dns-packages.tsv && \\
    rm -rf /tmp/debs /var/lib/apt/lists/*
""".format(base=lock["base"])
    (args.output / "Dockerfile").write_text(dockerfile)
    command = ["docker", "build", "--network", "none", "--platform", args.platform,
               "--iidfile", str(args.output / "image.id"), str(args.output)]
    record["buildCommand"] = command
    manifest.write_text(json.dumps(record, indent=2) + "\n")
    subprocess.run(command, check=True)
    record["imageId"] = (args.output / "image.id").read_text().strip()
    record["status"] = "built"
    manifest.write_text(json.dumps(record, indent=2) + "\n")
    print(manifest)


if __name__ == "__main__":
    main()
