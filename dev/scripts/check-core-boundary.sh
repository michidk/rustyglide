#!/usr/bin/env bash
# rustyglide-core must stay free of GPU, window and OS crates so it can be
# tested without a GPU and reused by any renderer.
set -euo pipefail
cd "$(dirname "$0")/../.."
tree=$(cargo tree -p rustyglide-core --edges normal,build --prefix none)
if grep -E '^(wgpu|wgpu-[a-z]+|naga|winit|raw-window-handle|sdl2|windows-sys) ' <<<"$tree"; then
    echo "rustyglide-core depends on a GPU, window or OS crate; keep those in rustyglide-wgpu or the front ends" >&2
    exit 1
fi
echo "core boundary: ok"
