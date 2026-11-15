#!/usr/bin/env bash
# Runs the GPU tests on lavapipe (software Vulkan) in the dev/docker image.
# Test binaries are built on the host; the repository is mounted at the same
# path so their compiled-in paths (golden images) resolve. Extra arguments go
# to the test binaries, e.g. `dev/scripts/gpu-test.sh golden`.
set -euo pipefail
cd "$(dirname "$0")/../.."
root=$PWD
docker image inspect rustyglide-test:dev >/dev/null 2>&1 || docker build -q -t rustyglide-test:dev dev/docker
mapfile -t binaries < <(cargo test -p rustyglide-scenes --no-run --message-format=json 2>/dev/null \
    | python3 -c 'import json,sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("profile", {}).get("test") and m.get("executable"):
        print(m["executable"])')
mkdir -p dev/out
status=0
for bin in "${binaries[@]}"; do
    cp "$bin" dev/out/
    name=$(basename "$bin")
    docker run --rm --user "$(id -u):$(id -g)" -v "$root:$root" -w "$root" \
        -e RUSTYGLIDE_BLESS -e WGPU_BACKEND="${WGPU_BACKEND:-vulkan}" -e RUST_BACKTRACE=1 \
        rustyglide-test:dev "$root/dev/out/$name" "$@" || status=1
done
rm -rf dev/out
exit $status
