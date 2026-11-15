//! The `guTex*` texture memory manager.
//!
//! Glide 2 utilities allocate texture memory linearly per TMU and describe each
//! texture with a `GrMipMapInfo` the game can read through a pointer, so the
//! records keep the C layout and a stable address.

use crate::Context;
use crate::texture::{self, NccData as Ncc, TexInfo};
use crate::types::*;
use std::ffi::c_void;

/// `GuNccTable` in its C layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct GuNccTable {
    pub y_rgb: [u8; 16],
    pub i_rgb: [[i16; 3]; 4],
    pub q_rgb: [[i16; 3]; 4],
    pub packed_data: [u32; 12],
}

impl GuNccTable {
    pub fn from_table(t: &Ncc) -> Self {
        let mut out = Self { y_rgb: t.y, i_rgb: t.i, q_rgb: t.q, packed_data: [0; 12] };
        for k in 0..4 {
            out.packed_data[k] = u32::from_le_bytes([t.y[k * 4], t.y[k * 4 + 1], t.y[k * 4 + 2], t.y[k * 4 + 3]]);
            let pack =
                |v: [i16; 3]| ((v[0] as u32 & 0x1ff) << 18) | ((v[1] as u32 & 0x1ff) << 9) | (v[2] as u32 & 0x1ff);
            out.packed_data[k + 4] = pack(t.i[k]);
            out.packed_data[k + 8] = pack(t.q[k]);
        }
        out
    }

    pub fn table(&self) -> Ncc {
        Ncc { y: self.y_rgb, i: self.i_rgb, q: self.q_rgb }
    }
}

/// `GrMipMapInfo` in its C layout.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GrMipMapInfo {
    pub sst: i32,
    pub valid: i32,
    pub width: i32,
    pub height: i32,
    pub aspect_ratio: i32,
    pub data: *mut c_void,
    pub format: i32,
    pub mipmap_mode: i32,
    pub magfilter_mode: i32,
    pub minfilter_mode: i32,
    pub s_clamp_mode: i32,
    pub t_clamp_mode: i32,
    pub t_lod: u32,
    pub t_texture_mode: u32,
    pub lod_bias: u32,
    pub lod_min: i32,
    pub lod_max: i32,
    pub tmu: i32,
    pub odd_even_mask: u32,
    pub tmu_base_address: u32,
    pub trilinear: i32,
    pub ncc_table: GuNccTable,
}

/// `GR_NULL_MIPMAP_HANDLE`.
pub const NULL_MIPMAP: u32 = u32::MAX;

/// Arguments of `guTexAllocateMemory` and `guTexChangeAttributes`.
#[derive(Debug, Clone, Copy)]
pub struct Attributes {
    pub width: i32,
    pub height: i32,
    pub format: TextureFormat,
    pub mipmap: MipMapMode,
    pub small_lod: Lod,
    pub large_lod: Lod,
    pub aspect: Aspect,
    pub clamp_s: ClampMode,
    pub clamp_t: ClampMode,
    pub min_filter: FilterMode,
    pub mag_filter: FilterMode,
}

#[derive(Default)]
pub struct Manager {
    /// Boxed so each record keeps its address: games hold pointers to them.
    #[allow(clippy::vec_box)]
    infos: Vec<Box<GrMipMapInfo>>,
    free: [u32; 2],
    current: [Option<u32>; 2],
}

/// Textures may not straddle the 2 MB boundary of a TMU.
const BOUNDARY: u32 = 2 << 20;

fn info_of(m: &GrMipMapInfo) -> Option<TexInfo> {
    Some(TexInfo {
        small_lod: Lod::try_from(m.lod_min as u32).ok()?,
        large_lod: Lod::try_from(m.lod_max as u32).ok()?,
        aspect: Aspect::try_from(m.aspect_ratio as u32).ok()?,
        format: TextureFormat::try_from(m.format as u32).ok()?,
    })
}

impl Context {
    pub fn gu_tex_mem_reset(&mut self) {
        self.gutex = Manager::default();
    }

    pub fn gu_tex_mem_query_avail(&self, tmu: Tmu) -> u32 {
        let total = self.tex_max_address(tmu) + 8;
        total.saturating_sub(self.gutex.free.get(tmu.index()).copied().unwrap_or(total))
    }

    /// `guTexAllocateMemory`. Returns a handle or [`NULL_MIPMAP`].
    pub fn gu_tex_allocate(
        &mut self,
        tmu: Tmu,
        even_odd: u32,
        attrs: Attributes,
        lod_bias: f32,
        trilinear: bool,
    ) -> u32 {
        let size = texture::mem_required(attrs.small_lod, attrs.large_lod, attrs.aspect, attrs.format, even_odd);
        let total = self.tex_max_address(tmu) + 8;
        let Some(free) = self.gutex.free.get_mut(tmu.index()) else { return NULL_MIPMAP };
        let mut start = *free;
        if start < BOUNDARY && start + size > BOUNDARY {
            start = BOUNDARY;
        }
        if start + size > total {
            return NULL_MIPMAP;
        }
        *free = start + size;
        let mut info = GrMipMapInfo {
            sst: 0,
            valid: 1,
            width: 0,
            height: 0,
            aspect_ratio: 0,
            data: std::ptr::null_mut(),
            format: 0,
            mipmap_mode: 0,
            magfilter_mode: 0,
            minfilter_mode: 0,
            s_clamp_mode: 0,
            t_clamp_mode: 0,
            t_lod: 0,
            t_texture_mode: 0,
            lod_bias: (lod_bias * 4.0).round() as i32 as u32,
            lod_min: 0,
            lod_max: 0,
            tmu: tmu as i32,
            odd_even_mask: even_odd,
            tmu_base_address: start,
            trilinear: trilinear as i32,
            ncc_table: GuNccTable::default(),
        };
        apply(&mut info, &attrs);
        self.gutex.infos.push(Box::new(info));
        (self.gutex.infos.len() - 1) as u32
    }

    /// `guTexChangeAttributes`: fails when the new size does not fit the allocation.
    pub fn gu_tex_change_attributes(&mut self, id: u32, attrs: Attributes) -> bool {
        let Some(info) = self.gutex.infos.get_mut(id as usize) else { return false };
        let Some(old) = info_of(info) else { return false };
        let old_size = texture::mem_required(old.small_lod, old.large_lod, old.aspect, old.format, info.odd_even_mask);
        let new_size =
            texture::mem_required(attrs.small_lod, attrs.large_lod, attrs.aspect, attrs.format, info.odd_even_mask);
        if new_size > old_size {
            return false;
        }
        apply(info, &attrs);
        true
    }

    pub fn gu_tex_info(&mut self, id: u32) -> Option<&mut GrMipMapInfo> {
        self.gutex.infos.get_mut(id as usize).map(|b| &mut **b)
    }

    pub fn gu_tex_current(&self, tmu: Tmu) -> u32 {
        self.gutex.current.get(tmu.index()).copied().flatten().unwrap_or(NULL_MIPMAP)
    }

    /// `guTexDownloadMipMap`: every level from `data`, plus the NCC table for YIQ formats.
    pub fn gu_tex_download_mipmap(&mut self, id: u32, data: &[u8], ncc: Option<&Ncc>) {
        let Some(m) = self.gutex.infos.get_mut(id as usize) else { return };
        let Some(info) = info_of(m) else { return };
        if info.format.uses_ncc()
            && let Some(table) = ncc
        {
            m.ncc_table = GuNccTable::from_table(table);
        }
        let (tmu, address, even_odd) = (m.tmu as u32, m.tmu_base_address, m.odd_even_mask);
        let Ok(tmu) = Tmu::try_from(tmu) else { return };
        self.tex_download_mipmap(tmu, address, even_odd, info, data);
    }

    /// `guTexDownloadMipMapLevel`: one level; returns the bytes it consumed.
    pub fn gu_tex_download_level(&mut self, id: u32, lod: Lod, data: &[u8]) -> usize {
        let Some(m) = self.gutex.infos.get(id as usize) else { return 0 };
        let Some(info) = info_of(m) else { return 0 };
        let Ok(tmu) = Tmu::try_from(m.tmu as u32) else { return 0 };
        let (address, even_odd) = (m.tmu_base_address, m.odd_even_mask);
        self.tex_download_level(tmu, address, lod, info.large_lod, info.aspect, info.format, even_odd, data);
        (texture::level_host_texels(lod, info.aspect) * info.format.bytes_per_texel()) as usize
    }

    /// `guTexSource`: binds a texture and its sampling state.
    pub fn gu_tex_source(&mut self, id: u32) {
        let Some(m) = self.gutex.infos.get(id as usize).map(|b| **b) else { return };
        let (Some(info), Ok(tmu)) = (info_of(&m), Tmu::try_from(m.tmu as u32)) else { return };
        let get = |v: i32| u32::try_from(v).ok();
        self.tex_source(tmu, m.tmu_base_address, m.odd_even_mask, info);
        self.tex_filter_mode(
            tmu,
            get(m.minfilter_mode).and_then(|v| v.try_into().ok()).unwrap_or_default(),
            get(m.magfilter_mode).and_then(|v| v.try_into().ok()).unwrap_or_default(),
        );
        self.tex_clamp_mode(
            tmu,
            get(m.s_clamp_mode).and_then(|v| v.try_into().ok()).unwrap_or_default(),
            get(m.t_clamp_mode).and_then(|v| v.try_into().ok()).unwrap_or_default(),
        );
        self.tex_mipmap_mode(
            tmu,
            get(m.mipmap_mode).and_then(|v| v.try_into().ok()).unwrap_or_default(),
            m.trilinear != 0,
        );
        self.tex_lod_bias(tmu, m.lod_bias as i32 as f32 / 4.0);
        if info.format.uses_ncc() {
            self.tex_download_ncc(tmu, NccTable::Ncc0, &m.ncc_table.table());
            self.tex_ncc_table(tmu, NccTable::Ncc0);
        }
        if let Some(slot) = self.gutex.current.get_mut(tmu.index()) {
            *slot = Some(id);
        }
    }
}

fn apply(info: &mut GrMipMapInfo, a: &Attributes) {
    info.width = a.width;
    info.height = a.height;
    info.format = a.format as i32;
    info.mipmap_mode = a.mipmap as i32;
    info.lod_min = a.small_lod as i32;
    info.lod_max = a.large_lod as i32;
    info.aspect_ratio = a.aspect as i32;
    info.s_clamp_mode = a.clamp_s as i32;
    info.t_clamp_mode = a.clamp_t as i32;
    info.minfilter_mode = a.min_filter as i32;
    info.magfilter_mode = a.mag_filter as i32;
}
