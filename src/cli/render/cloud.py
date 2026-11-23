"""Ephemeral Modal app for `render --cloud`: runs one render in a container and returns the MP4.

Started by `modal run` from the CLI; everything arrives through environment variables and
arguments, and the app stops as soon as the file is written.
"""
import json
import os
import pathlib
import subprocess
import sys

import modal

# The container is always Linux x64, whatever machine started the render. It runs the release
# build of this very version, or an explicit Linux binary in AETHER_CLOUD_BIN (for unreleased builds).
# The image is only described on the machine that starts the render: Modal re-imports this file
# inside the container, where the AETHER_CLOUD_* variables do not exist, so reading them there
# would crash the container on start (and Modal would restart it forever).
if modal.is_local():
    base = modal.Image.debian_slim().apt_install("curl")
    if os.environ.get("AETHER_CLOUD_BIN"):
        base = base.add_local_file(os.environ["AETHER_CLOUD_BIN"], "/usr/local/bin/aether-motion", copy=True)
    else:
        tag = "v" + os.environ["AETHER_CLOUD_VERSION"]
        base = base.run_commands(
            "curl -fsSL -o /tmp/am.tgz https://github.com/ferxalb/aether-motion/releases/download/"
            f"{tag}/aether-motion-{tag}-x86_64-unknown-linux-gnu.tar.gz",
            "mkdir /tmp/am && tar -xzf /tmp/am.tgz -C /tmp/am",
            "install -m 755 $(find /tmp/am -name aether-motion -type f | head -n 1) /usr/local/bin/aether-motion",
        )
    image = base.run_commands("chmod +x /usr/local/bin/aether-motion").add_local_dir(
        os.environ["AETHER_CLOUD_BUNDLE"], "/job"
    )
else:
    image = modal.Image.debian_slim()
app = modal.App("aether-render", image=image)


@app.function(cpu=float(os.environ.get("AETHER_CLOUD_CPU", "8")), timeout=3600, retries=0)
def render(args):
    print("container started, rendering", file=sys.stderr, flush=True)
    proc = subprocess.Popen(
        ["aether-motion", *args, "-o", "/tmp/out.mp4", "--force", "--progress", "json"],
        cwd="/job",
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    tail = []
    for line in proc.stderr:
        if line.startswith("{"):
            yield ("p", line.strip())
        else:
            tail = (tail + [line])[-20:]
    if proc.wait() != 0:
        yield ("e", "".join(tail))
    else:
        yield ("v", pathlib.Path("/tmp/out.mp4").read_bytes())


@app.local_entrypoint()
def main(args: str, out: str):
    print("uploading bundle and preparing the remote image (the first run can take minutes)", file=sys.stderr, flush=True)
    for kind, data in render.remote_gen(json.loads(args)):
        if kind == "p":
            print(data, file=sys.stderr, flush=True)
        elif kind == "e":
            sys.stderr.write(data)
            sys.exit(1)
        else:
            pathlib.Path(out).write_bytes(data)
