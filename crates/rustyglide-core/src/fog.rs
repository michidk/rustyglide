//! Fog tables, the `guFog*` helpers, and the hardware's 16-bit floating W.
//!
//! The W-buffer and table fog both use the same 16-bit "wfloat" encoding of the
//! per-pixel 1/w: four bits of exponent and twelve of inverted mantissa. Table
//! fog indexes with its top six bits and interpolates with the next eight. The
//! shader computes the same value; this is the reference.

use crate::types::FOG_TABLE_SIZE;

pub type FogTable = [u8; FOG_TABLE_SIZE];

/// `guFogTableIndexToW`: the eye-space W a table entry stands for.
pub fn index_to_w(i: usize) -> f32 {
    (2f64.powf(3.0 + (i >> 2) as f64) / (8 - (i & 3)) as f64) as f32
}

fn generate(table: &mut FogTable, f: impl Fn(usize) -> f32) {
    for (i, entry) in table.iter_mut().enumerate() {
        *entry = (f(i).clamp(0.0, 1.0) * 255.0) as u8;
    }
}

/// `guFogGenerateExp`.
pub fn generate_exp(table: &mut FogTable, density: f32) {
    let dp = density * index_to_w(FOG_TABLE_SIZE - 1);
    let scale = 1.0 / (1.0 - (-dp).exp());
    generate(table, |i| (1.0 - (-(density * index_to_w(i))).exp()) * scale);
}

/// `guFogGenerateExp2`.
pub fn generate_exp2(table: &mut FogTable, density: f32) {
    let dp = density * index_to_w(FOG_TABLE_SIZE - 1);
    let scale = 1.0 / (1.0 - (-(dp * dp)).exp());
    generate(table, |i| {
        let dp = density * index_to_w(i);
        (1.0 - (-(dp * dp)).exp()) * scale
    });
}

/// `guFogGenerateLinear`.
pub fn generate_linear(table: &mut FogTable, near: f32, far: f32) {
    generate(table, |i| (index_to_w(i).min(65535.0) - near) / (far - near));
}

/// The hardware's 16-bit floating encoding of `oow` (1/w): 0 is nearest, 0xffff farthest.
pub fn wfloat(oow: f32) -> u16 {
    if oow.is_nan() || oow >= 1.0 || oow <= 0.0 {
        // Integer part set (or behind the eye): nearest.
        return if oow > 0.0 || oow.is_nan() { 0 } else { 0xffff };
    }
    if oow < 1.0 / 65536.0 {
        return 0xffff;
    }
    let bits = oow.to_bits();
    let exp = 127 - 1 - ((bits >> 23) & 0xff) as i32;
    let inverted = !(bits >> 11) & 0xfff;
    (((exp as u32) << 12 | inverted) + 1).min(0xffff) as u16
}

/// Table fog density for a pixel with the given `oow`, in 0..=255.
pub fn table_lookup(table: &FogTable, oow: f32) -> f32 {
    let w = wfloat(oow) as usize;
    let index = w >> 10;
    let frac = ((w >> 2) & 0xff) as f32;
    let this = table[index] as f32;
    let next = table[(index + 1).min(FOG_TABLE_SIZE - 1)] as f32;
    this + (next - this) * frac / 256.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_to_w_matches_formula() {
        assert_eq!(index_to_w(0), 1.0);
        assert_eq!(index_to_w(4), 2.0);
        assert!((index_to_w(63) - 262144.0 / 5.0).abs() < 0.01);
    }

    #[test]
    fn wfloat_is_monotonic_with_distance() {
        assert_eq!(wfloat(1.0), 0);
        assert_eq!(wfloat(1.0 / 100000.0), 0xffff);
        let mut last = 0;
        for i in 1..2000 {
            let w = wfloat(1.0 / (1.0 + i as f32 * 17.0));
            assert!(w >= last, "wfloat must grow with w");
            last = w;
        }
    }

    #[test]
    fn table_index_matches_index_to_w() {
        // A pixel at exactly an entry's W must land on that entry with no fraction.
        for i in 1..63 {
            let w = wfloat(1.0 / index_to_w(i)) as usize;
            assert_eq!(w >> 10, i, "entry {i}");
        }
    }

    #[test]
    fn linear_table_ramps() {
        let mut t = [0; FOG_TABLE_SIZE];
        generate_linear(&mut t, 0.0, 1000.0);
        assert_eq!(t[0], 0);
        assert_eq!(t[63], 255);
        assert!(t.windows(2).all(|w| w[0] <= w[1]));
    }
}
