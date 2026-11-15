# rustyglide design

rustyglide implements the 3dfx Glide API in Rust on wgpu (Vulkan, Metal, DX12,
OpenGL fallback). It is a library first: a Rust program can link it and call
Glide in process, and thin front ends build the `glide2x`/`glide3x` drop-in
libraries that old games load.

- **Target:** feature parity with OpenGLide-ng for Glide 2.4x-2.6x
  (`glide2x`), then Glide 3 (`glide3x`) and the Glide 2.1x `glide.dll`.
- **First consumer:** Maiko (Redguard). Redguard is the only game tested until
  it runs correctly. Every structure below must still serve any Glide game.

## Workspace

```
crates/
  rustyglide-core/    Glide semantics. No GPU, window or OS types.
  rustyglide-wgpu/    The renderer: consumes core output, owns wgpu objects.
  rustyglide/         Public Rust API: Glide = core + renderer + target.
  glide2x/            cdylib: Glide 2.4x C ABI over `rustyglide`.
  glide3x/            cdylib, later: Glide 3 front end.
  glide1x/            cdylib, later: Glide 2.1x shim (glide.dll).
  rustyglide-scenes/  Test scenes, golden images and the trace format.
dev/                  gates.sh, reference-renderer container, scripts.
```

### rustyglide-core

All Glide behaviour that does not need a GPU:

- **Types and constants.** Every enum, struct layout and ABI fact, typed
  (`GrCombineFunction` and similar), with checked conversion from raw `u32`.
  Unknown values are errors that the front ends log, never silent defaults.
- **State machine.** The full `GrState` and its 2.1x and 2.x sizes, plus the
  get/set state calls.
- **Texture memory.** A byte array per TMU, sized like the hardware or by
  config. Downloads, partial downloads, multibase, `grTexMinAddress` and
  `grTexMaxAddress`, `grTexCalcMemRequired` and the `guTex*` memory manager
  all work on these bytes, as on a real board.
- **Texture decoding.** Every format to RGBA8: 332, 8332, 565, 1555, 4444,
  A8, I8, AI44, AI88, P8, AP88, YIQ (NCC), AYIQ, with 565 and 6666 palettes.
- **Fog.** Fog tables, the `guFog*` generators and `guFogTableIndexToW`.
- **Vertices.** `GrVertex` decoding: snap-bias removal, `ooz`/`oow`, s/t
  scaled to 256 along the long axis. Polygon and planar-polygon fans, the
  `gu*WithClip` variants with real clipping, and the 3DF loader.
- **LFB.** Framebuffer pixel packing and unpacking for every write mode, origin
  flips and region reads and writes, all on CPU buffers.
- **Commands.** Core turns calls into a frame of plain-data commands: draw a
  batch with a state snapshot, clear, upload a texture region, read or write
  the LFB, swap. The renderer consumes these.

There is one renderer, so core produces data instead of calling a backend
trait. That keeps core unit-testable without a GPU and lets the same command
stream be recorded and replayed (see Testing). A boundary check (as in Cyrus,
`check-core-boundaries.sh`) fails CI if core gains `wgpu`, `winit` or
`raw-window-handle` in its dependency tree.

### rustyglide-wgpu

- **One fixed-function shader in WGSL.** It covers:
  - TMU chaining (`grTexCombine` for TMU1 into TMU0);
  - color and alpha combine, including LOCAL_DEPTH, LOD_FRACTION and
    detail factors, and `grAlphaControlsITRGBLighting`;
  - chroma key, with 8-bit integer alpha test and chroma compare;
  - table and iterated fog;
  - 4x4 and 2x2 dithering;
  - Z and W buffering, depth bias.
- **State per draw.** Combine, fog, alpha-test and chroma state go in a
  uniform block with a dynamic offset. Push constants are not portable to
  every wgpu backend.
- **Pipelines.** Pipelines differ only by blend, depth, cull and color-mask
  state. They are cached by that key and created lazily.
- **Batching.** Triangles collect in a vertex buffer. A batch ends when the
  pipeline key, a texture binding or the state block changes.
- **Interpolation as on the hardware.** Voodoo iterates color and alpha
  linearly in screen space and divides s/w, t/w by 1/w per pixel. The shader
  interpolates everything with `@interpolate(linear)` and divides in the
  fragment stage. That reproduces the hardware exactly instead of using GPU
  perspective correction.
- **Texture cache.** Decoded textures are keyed by TMU address range, texture
  info and palette or NCC generation. A download that overlaps a range
  invalidates it, and content hashes avoid re-uploading unchanged data
  (GXVK's model).
- **Render target.** The game renders offscreen at its Glide resolution, times
  a configurable integer scale. `present` scales the result into the surface
  with the aspect ratio kept (4:3 with bars by default), then runs a present
  hook with the device, queue, encoder and surface view so a host can draw an
  overlay (egui-wgpu in Maiko).
- **LFB.** Dirty flags track whether the CPU or GPU copy is current, as in
  GXVK.
  - A read lock copies the target to a buffer and waits for the map, and only
    when the GPU copy changed since the last read.
  - A write lock gives a CPU shadow that keeps the existing pixels.
  - Unlock uploads the changed rows and draws them through the pixel pipeline
    or straight to the target, as the lock requests.
- **Gamma** is applied in the present pass, never per fragment.

### rustyglide (public API)

```rust
let mut glide = Glide::new(Config::from_env())?;
glide.sst_win_open(Target::window(&sdl_window), GrResolution::R640x480, ..)?;
glide.color_combine(..); glide.draw_triangle(&a, &b, &c);
glide.buffer_swap(1);
glide.set_present_hook(|frame| overlay.paint(frame));
```

Methods mirror Glide calls one to one with Rust types.

Targets:
- any `HasWindowHandle + HasDisplayHandle` window (SDL2 0.38 implements
  raw-window-handle 0.6 and ships a wgpu example);
- a headless texture, for tests;
- a window rustyglide creates itself with winit, pumping events on swap. This
  is for the drop-in libraries when the game does not give one.

Configuration is a TOML file plus `RUSTYGLIDE_*` variables. It covers the
OpenGLide-ng options that still mean something: resolution scale, forced
filtering, mipmaps on or off, vsync, texture memory size, ignore palette
change, aspect handling and the splash screen. Backend selection is wgpu's own
`WGPU_BACKEND`.

### Front ends (glide2x, glide3x, glide1x)

- **One export table per DLL** in Rust: name, argument bytes, handler.
  `build.rs` generates the `.def` file with the decorated stdcall names
  (`_grDrawTriangle@12`), so names and stack sizes cannot drift (wildart's
  approach). 16-bit arguments arrive in 32-bit slots and are masked.
- **Calling convention.** `extern "system"`, which is stdcall on i686 Windows
  and C elsewhere.
- **Global state.** One global `Glide` behind a lock, created by
  `grGlideInit`. Glide is single-threaded, so the lock never contends.
- **Glide 3** decodes `grVertexLayout`, `grDrawVertexArray*`, coordinate space,
  viewport and `grGet`. It converts the inverted LOD and aspect enums and maps
  everything else onto the same core.
- **Glide 2.1x** maps `grSstOpen`, `grLfbBegin/End/GetReadPtr` and the other
  2.1x calls onto the 2.4x paths, and uses the 2.1x `GrState` size (GXVK's
  shim pattern).
- **Version string.** `grGlideGetVersion` reports a real version string for
  each flavour, because games check it.

## Testing

| Layer | What runs | Where |
|---|---|---|
| Unit | Core: format decoding, fog, vertex decoding, LFB packing, texture-memory accounting, enum conversions | every CI run |
| Combiner check | A CPU implementation of combine, fog, alpha test and chroma key (semantics from the 3dfx SDK sources and the BSD-3 MAME Voodoo core) against the WGSL shader on single-pixel draws, over a sweep of state combinations | every CI run, lavapipe |
| Scenes | Rust programs, one per feature, written from the SDK's test descriptions (not translated from its sources): clear, gouraud, Z, W, blending, fog, chroma key, culling, LFB origin and render buffer, depth bias, clip window, mipmaps and trilinear, texture compositing, palettes, multibase, partial LOD downloads. Rendered headless and compared to golden PNGs with a per-channel tolerance | every CI run, lavapipe |
| Parity | The same scenes through OpenGLide-ng's `libglide2x.so` (built only in a dev container, with llvmpipe and Xvfb, never linked or shipped) and through our `glide2x`. The scene runner loads either library over the same C ABI. Differences above the tolerance are listed and either fixed or recorded as a deliberate deviation. This is also how goldens are made | on demand, dev container |
| Traces | Maiko records Redguard's Glide command stream in the core command format; replay renders it headless and compares frames to OpenGLide-ng. Traces contain game data and stay local, never in Git or CI | local, with an owned copy |
| Real hardware | Maiko on Windows, Linux and macOS GPUs | before calling a milestone done |

CI uses lavapipe because it is deterministic and needs no GPU. Windows CI can
run the scenes on WARP (D3D12's software rasterizer); macOS runs them on Metal
if the hosted runner exposes it, otherwise it only builds.

## CI

Modelled on Cyrus, with actions pinned by SHA and the toolchain pinned in
`rust-toolchain.toml`:

- **`ci.yml`** on push to `main` and on pull requests:
  - **Lints:** `cargo fmt --check`, `clippy --all-targets -D warnings`, the
    core boundary check, `cargo deny` (licences: reject GPL and LGPL in the
    production tree; advisories).
  - **Test:** unit tests, the combiner check and the scenes on Ubuntu with
    `mesa-vulkan-drivers`, with scene diffs uploaded as an artifact on
    failure.
  - **Build matrix:**
    - `x86_64-unknown-linux-gnu`
    - `x86_64-pc-windows-msvc`
    - `i686-pc-windows-msvc`, the 32-bit `glide2x.dll` most real games need
    - `aarch64-apple-darwin` and `x86_64-apple-darwin`
    - Windows runs the scenes on WARP.
- **`pr-title.yml`:** Conventional Commits check, as in Cyrus.
- **`bump-version.yml` and `release.yml`:** git-cliff versioning and
  per-target `glide2x` archives with `SHA256SUMS`, as in Cyrus.
- **Renovate** with the shared `michidk/workflows` preset.
- **Locally:** `dev/scripts/gates.sh` runs the same stages, behind a
  `justfile` (`just gates`, `just verify <crate>`, `just parity`).

## What we take from existing projects

| Source | Licence | Taken |
|---|---|---|
| lutris/GlideVK | MIT OR Apache-2.0 | Code may be adapted with attribution: blend-factor permutation table, Glide 2 `GrVertex` offsets and snap removal, depth mapping, resolution table, the documented quirks (default blend ONE/ZERO, s/t span, palettes resolved when drawing). The idea of diffing test programs against a reference DLL. Not taken: its combine functions 0x9/0x10 (wrong), per-draw submission, float alpha test, per-fragment gamma. |
| GXVK | zlib | Code may be adapted with notice: per-TMU byte memory with a region map and hash-based re-upload, LFB CPU/GPU dirty tracking, combiner structure (TMU chain, LOCAL_DEPTH, ITRGB lighting), LOD and aspect conversion between 2.x and 3.x, export lists for `glide.dll`/`glide2x.dll`/`glide3x.dll`, `GrState` sizes, the 2.1x shim. Avoid its `grLfbReadRegion` stride bug and float alpha test. |
| wildart/glidevk | none, all rights reserved | Ideas only: `.def` generated from an export table, Glide's fog W formula, LFB write locks that keep existing pixels, batching that does not split on combine changes. |
| rust-dos | GPL-2.0-or-later | Ideas only: combine-factor bit semantics, odd-LOD trilinear detection, its per-version export sets as a cross-check. |
| MAME Voodoo core | BSD-3-Clause | The per-pixel reference for the CPU combiner check. |
| OpenGLide-ng | LGPL | Not linked or copied. The parity target and the test oracle: its feature list, config options, format list, 3DF and `guTex*` behaviour, and the fixes Maiko patched in (fog in 1/w, A8 luminance, no fog on LFB writes). |
| 3dfx Glide SDK | 3DFX Glide Source Code GPL | The authority on semantics, read for facts. Constants and layouts are re-stated as interface facts. Its code and tests are not translated; our scenes are written from its test descriptions. |

Attribution for adapted code goes in `THIRD_PARTY.md` and in a comment at
each adapted item. The crate licence is MIT OR Apache-2.0, compatible with
Maiko's MIT and with both copyable sources.

## Parity checklist

OpenGLide-ng exports 129 Glide 2 calls. rustyglide implements every call it
implements, with the same or better behaviour, and treats its stubs as follows:

- **Implement properly where OpenGLide-ng only half does:**
  - real clipping in `gu*WithClip`;
  - `grTexMultibase` and `grTexMultibaseAddress`, `grTexDetailControl`,
    `grTexLodBiasValue`, `grAlphaControlsITRGBLighting`;
  - `grLfbConstantAlpha`, `grLfbConstantDepth`, `grLfbWriteColorFormat`,
    `grLfbWriteColorSwizzle`;
  - `guEncodeRLE16`;
  - `ConvertAndDownloadRle` on 64-bit.
- **Keep as accepted no-ops:** performance and triangle statistics,
  `grSstControl`, `grSstVideoLine`, `guMovie*`.
- **Fake as needed:** `grSstStatus`, `grSstVRetraceOn` and `grSstIsBusy`
  report a plausible board state.
- **Antialiased primitives** draw as their normal versions at first, as
  OpenGLide-ng does. Real edge AA comes after parity.
- **Splash and banner** need 3dfx's model data, which is not ours to ship.
  `grSplash` is an accepted no-op; it is off by default in OpenGLide-ng too.

## Maiko integration

Redguard imports 124 Glide 2 calls, and Maiko forwards 37 of them today.

1. **Replace the renderer.** `runtime/glide/lib.rs` calls `rustyglide` instead
   of the `extern` block. The renderer becomes an ordinary Rust dependency, so
   the `redguard-glide` library, its build scripts, the three OpenGLide patches
   and the LGPL source tarball in the kits are removed.
2. **Window.** The SDL window is created without an OpenGL context and passed
   as a raw window handle.
3. **Overlay.** egui moves from `egui_glow` to `egui-wgpu` and draws from the
   present hook.
4. **Behaviour kept from the patches:**
   - offscreen rendering scaled to a resizable window with 4:3 bars;
   - per-pixel fog in 1/w;
   - full luminance for A8 textures;
   - no fog on LFB writes;
   - clamping while a movie plays;
   - 60 Hz swap pacing.

   The last two stay in Maiko's bridge; the rest are rustyglide behaviour.
5. **Host-side reads.** Screenshots, `RG_CAPTURE_DIR` and the flat-frame
   detection keep using a read lock on the back buffer.

## Milestones

1. **Skeleton.** Workspace, CI, core types, the `glide2x` export table with
   every call present (unimplemented calls log once), headless target and
   golden-scene harness.
2. **Untextured rendering.** Device, offscreen target and present; clear and
   swap; triangles, lines and points; depth Z and W; blending, cull, color
   mask, clip window, constant and iterated color.
3. **Textures.** TMU memory and every format, palettes and NCC; filter,
   clamp, mipmaps and LOD bias; all combiners, chroma key, alpha test, fog,
   dithering.
4. **The rest of the API.** LFB lock, read and write, regions and every write
   mode; gamma; get and set state; `gu*` helpers; the 3DF loader and the
   texture memory manager.
5. **Maiko.** Integration and Redguard trace parity, then real-hardware runs
   on Windows, Linux and macOS.
6. **Drop-ins.** `glide2x.dll` for Windows and Wine (32 and 64-bit),
   `libglide2x.so` with its own window for native Linux and DOSBox/dosemu
   builds, then `glide.dll` (2.1x) and `glide3x`.
