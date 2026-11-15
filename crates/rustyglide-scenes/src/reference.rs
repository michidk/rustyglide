//! A CPU model of the FBI color and alpha combine units, written from the
//! Glide 2 reference: `other` (unless zeroed) minus `local`, scaled by the
//! factor, plus `local` or local alpha, clamped, optionally inverted.
//! Tests compare the GPU shader with it.

use rustyglide::combine::Combine;
use rustyglide::types::{CombineFactor, CombineFunction, CombineLocal, CombineOther};

/// Per-pixel inputs to the combine units, 0..=1.
#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    pub iterated: [f32; 4],
    pub constant: [f32; 4],
    pub texel: [f32; 4],
}

struct Parts {
    zero_other: bool,
    sub_local: bool,
    add_local: bool,
    add_local_alpha: bool,
}

fn parts(f: CombineFunction) -> Parts {
    use CombineFunction as F;
    Parts {
        zero_other: matches!(
            f,
            F::Zero | F::Local | F::LocalAlpha | F::ScaleMinusLocalAddLocal | F::ScaleMinusLocalAddLocalAlpha
        ),
        sub_local: matches!(
            f,
            F::ScaleOtherMinusLocal
                | F::ScaleOtherMinusLocalAddLocal
                | F::ScaleOtherMinusLocalAddLocalAlpha
                | F::ScaleMinusLocalAddLocal
                | F::ScaleMinusLocalAddLocalAlpha
        ),
        add_local: matches!(f, F::Local | F::ScaleOtherAddLocal | F::ScaleOtherMinusLocalAddLocal | F::ScaleMinusLocalAddLocal),
        add_local_alpha: matches!(
            f,
            F::LocalAlpha | F::ScaleOtherAddLocalAlpha | F::ScaleOtherMinusLocalAddLocalAlpha | F::ScaleMinusLocalAddLocalAlpha
        ),
    }
}

fn unit(p: &Parts, factor: f32, local: f32, other: f32, local_alpha: f32, invert: bool) -> f32 {
    let mut c = if p.zero_other { 0.0 } else { other };
    if p.sub_local {
        c -= local;
    }
    c *= factor;
    if p.add_local {
        c += local;
    } else if p.add_local_alpha {
        c += local_alpha;
    }
    let c = c.clamp(0.0, 1.0);
    if invert { 1.0 - c } else { c }
}

fn one_minus(code: CombineFactor, f: f32) -> f32 {
    if (code as u32) & 8 != 0 { 1.0 - f } else { f }
}

/// The combined `[r, g, b, a]`.
pub fn shade(color: &Combine, alpha: &Combine, i: &Inputs) -> [f32; 4] {
    let a_local = match alpha.local {
        CombineLocal::Iterated => i.iterated[3],
        CombineLocal::Constant => i.constant[3],
        CombineLocal::Depth => 0.0,
    };
    let a_other = match alpha.other {
        CombineOther::Iterated => i.iterated[3],
        CombineOther::Texture => i.texel[3],
        CombineOther::Constant => i.constant[3],
    };
    let c_local = if color.local == CombineLocal::Constant { i.constant } else { i.iterated };
    let c_other = match color.other {
        CombineOther::Iterated => i.iterated,
        CombineOther::Texture => i.texel,
        CombineOther::Constant => i.constant,
    };
    let cp = parts(color.function);
    let mut out = [0.0; 4];
    for k in 0..3 {
        let base = match (color.factor as u32) & 7 {
            1 => c_local[k],
            2 => a_other,
            3 => a_local,
            4 => i.texel[3],
            5 => i.texel[k],
            _ => 0.0,
        };
        out[k] = unit(&cp, one_minus(color.factor, base), c_local[k], c_other[k], a_local, color.invert);
    }
    let ap = parts(alpha.function);
    let base = match (alpha.factor as u32) & 7 {
        1 | 3 => a_local,
        2 => a_other,
        4 => i.texel[3],
        _ => 0.0,
    };
    // Alpha has no separate local alpha: the `_ALPHA` functions add the local alpha.
    out[3] = unit(&ap, one_minus(alpha.factor, base), a_local, a_other, a_local, alpha.invert);
    out
}
