# Third-party references

rustyglide contains no code copied from other Glide implementations. These
sources informed it:

| Source | Licence | Use |
|---|---|---|
| [3dfx Glide SDK](https://github.com/sezero/glide) | 3DFX Glide Source Code General Public License | Interface facts: enumeration values, structure layouts, texture memory sizes, the `gu*` expansions and fog formulas. The scenes are written from its test descriptions, not translated from its sources. |
| [OpenGLide-ng](https://github.com/eugenenuke/openglide-ng) | LGPL | The parity target. Behaviour reference for its configuration, the sentinel-based LFB write lock and the Glidos run-length bitmap format of `ConvertAndDownloadRle`. Not linked or shipped. |
| [GXVK](https://github.com/CkNoSFeRaTU/gxvk) | Zlib | Design reference: per-TMU texture memory with hashed re-upload, LFB dirty tracking. |
| [GlideVK](https://github.com/lutris/GlideVK) | MIT OR Apache-2.0 | Design reference: documented Glide quirks, snap-bias removal. |

Rust dependencies and their licences are checked with `cargo deny`
(`deny.toml`); only permissive licences are allowed.
