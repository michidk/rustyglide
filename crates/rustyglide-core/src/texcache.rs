//! Decoded textures, keyed by where and how they sit in texture memory.

use crate::TexSource;
use crate::command::TextureId;
use crate::texture::{self, DecodeTables, TmuMemory};
use crate::types::{Lod, Tmu};
use std::collections::HashMap;
use std::ops::Range;

/// Textures kept before the least recently used is dropped.
const CAPACITY: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub tmu: u8,
    pub source: TexSource,
    /// Level base addresses when multibase addressing is on.
    pub multibase: Option<[u32; 4]>,
    /// Hash of the palette or NCC table the texture decodes with.
    pub table_hash: u64,
}

struct Entry {
    id: TextureId,
    range: Range<u32>,
    used: u64,
}

pub type Levels = Vec<(u32, u32, Vec<u8>)>;

#[derive(Default)]
pub struct TextureCache {
    entries: HashMap<Key, Entry>,
    next_id: TextureId,
    clock: u64,
    evicted: Vec<TextureId>,
}

/// Multibase range index of a level: 256, 128, 64, then 32 down to 1 share one base.
fn base_range(lod: Lod) -> usize {
    (lod as usize).min(3)
}

fn level_address(key: &Key, lod: Lod) -> u32 {
    let info = &key.source.info;
    match key.multibase {
        None => {
            key.source.address
                + texture::level_offset(lod, info.large_lod, info.aspect, info.format, key.source.even_odd)
        }
        Some(bases) => {
            let range = base_range(lod);
            let first = Lod::from_index((range as u32).max(info.large_lod as u32)).unwrap_or(lod);
            bases[range] + texture::level_offset(lod, first, info.aspect, info.format, key.source.even_odd)
        }
    }
}

fn memory_range(key: &Key) -> Range<u32> {
    let info = &key.source.info;
    let mut start = u32::MAX;
    let mut end = 0;
    for lod in (info.large_lod as u32..=info.small_lod as u32).filter_map(Lod::from_index) {
        let (w, h) = texture::level_dims(lod, info.aspect);
        let at = level_address(key, lod);
        start = start.min(at);
        end = end.max(at + (w * h).max(4) * info.format.bytes_per_texel());
    }
    start..end.max(start)
}

pub fn decode(memory: &TmuMemory, key: &Key, tables: &DecodeTables<'_>) -> Levels {
    texture::decode_chain_at(memory, key.source.even_odd, &key.source.info, tables, |lod| level_address(key, lod))
}

impl TextureCache {
    /// Returns the texture for `key`, decoding it with `make` when missing.
    /// The second value carries a newly created texture.
    pub fn get_or_insert(
        &mut self,
        key: Key,
        make: impl FnOnce() -> Levels,
    ) -> (TextureId, Option<(TextureId, Levels)>) {
        self.clock += 1;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.used = self.clock;
            return (entry.id, None);
        }
        if self.entries.len() >= CAPACITY
            && let Some((&oldest, _)) = self.entries.iter().min_by_key(|(_, e)| e.used)
            && let Some(entry) = self.entries.remove(&oldest)
        {
            self.evicted.push(entry.id);
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.entries.insert(key, Entry { id, range: memory_range(&key), used: self.clock });
        (id, Some((id, make())))
    }

    /// Drops textures of `tmu` that overlap `written`.
    pub fn invalidate(&mut self, tmu: Tmu, written: Range<u32>) -> Vec<TextureId> {
        let mut gone = Vec::new();
        self.entries.retain(|key, entry| {
            let hit =
                key.tmu as usize == tmu.index() && entry.range.start < written.end && written.start < entry.range.end;
            if hit {
                gone.push(entry.id);
            }
            !hit
        });
        gone
    }

    pub fn clear(&mut self) -> Vec<TextureId> {
        let mut gone: Vec<_> = self.entries.drain().map(|(_, e)| e.id).collect();
        gone.append(&mut self.evicted);
        gone
    }

    pub fn take_evicted(&mut self) -> Vec<TextureId> {
        std::mem::take(&mut self.evicted)
    }
}
