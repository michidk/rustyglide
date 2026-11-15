//! Combine-unit settings and the `gu*` shortcuts that expand to them.

use crate::types::{
    AlphaSource, ColorCombineFnc, CombineFactor, CombineFunction, CombineLocal, CombineOther, TextureCombineFnc,
};

/// Settings of the FBI color or alpha combine unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Combine {
    pub function: CombineFunction,
    pub factor: CombineFactor,
    pub local: CombineLocal,
    pub other: CombineOther,
    pub invert: bool,
}

impl Combine {
    pub const fn new(
        function: CombineFunction,
        factor: CombineFactor,
        local: CombineLocal,
        other: CombineOther,
        invert: bool,
    ) -> Self {
        Self { function, factor, local, other, invert }
    }

    /// Packs into the shader's layout: function (5 bits), factor (4), local (2), other (2), invert (1).
    pub fn pack(&self) -> u32 {
        self.function as u32
            | (self.factor as u32) << 5
            | (self.local as u32) << 9
            | (self.other as u32) << 11
            | (self.invert as u32) << 13
    }

    pub fn uses_texture(&self) -> bool {
        self.other == CombineOther::Texture
            || matches!(
                self.factor,
                CombineFactor::TextureAlpha
                    | CombineFactor::TextureRgb
                    | CombineFactor::OneMinusTextureAlpha
                    | CombineFactor::OneMinusLodFraction
            )
    }
}

/// Settings of a TMU's texture combine unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TexCombine {
    pub rgb_function: CombineFunction,
    pub rgb_factor: CombineFactor,
    pub alpha_function: CombineFunction,
    pub alpha_factor: CombineFactor,
    pub rgb_invert: bool,
    pub alpha_invert: bool,
}

impl TexCombine {
    pub const fn new(
        rgb_function: CombineFunction,
        rgb_factor: CombineFactor,
        alpha_function: CombineFunction,
        alpha_factor: CombineFactor,
        rgb_invert: bool,
        alpha_invert: bool,
    ) -> Self {
        Self { rgb_function, rgb_factor, alpha_function, alpha_factor, rgb_invert, alpha_invert }
    }

    /// Packs into the shader's layout: rgb function (5), rgb factor (4), alpha function (5),
    /// alpha factor (4), rgb invert (1), alpha invert (1).
    pub fn pack(&self) -> u32 {
        self.rgb_function as u32
            | (self.rgb_factor as u32) << 5
            | (self.alpha_function as u32) << 9
            | (self.alpha_factor as u32) << 14
            | (self.rgb_invert as u32) << 18
            | (self.alpha_invert as u32) << 19
    }

    /// Whether the unit reads this TMU's texel.
    pub fn uses_local(&self) -> bool {
        let reads = |function: CombineFunction, factor: CombineFactor| {
            !matches!(function, CombineFunction::Zero | CombineFunction::ScaleOther)
                || matches!(
                    factor,
                    CombineFactor::Local
                        | CombineFactor::LocalAlpha
                        | CombineFactor::OneMinusLocal
                        | CombineFactor::OneMinusLocalAlpha
                )
        };
        reads(self.rgb_function, self.rgb_factor) || reads(self.alpha_function, self.alpha_factor)
    }

    /// Whether the unit reads the upstream TMU's output.
    pub fn uses_other(&self) -> bool {
        let reads = |function: CombineFunction, factor: CombineFactor| {
            !matches!(function, CombineFunction::Zero | CombineFunction::Local | CombineFunction::LocalAlpha)
                && !matches!(function, CombineFunction::ScaleMinusLocalAddLocal | CombineFunction::ScaleMinusLocalAddLocalAlpha)
                || matches!(factor, CombineFactor::OtherAlpha | CombineFactor::OneMinusOtherAlpha)
        };
        reads(self.rgb_function, self.rgb_factor) || reads(self.alpha_function, self.alpha_factor)
    }
}

use CombineFactor as F;
use CombineFunction as Fn;
use CombineLocal as L;
use CombineOther as O;

/// Glide's state after `grSstWinOpen`: iterated color, constant alpha, TMUs output zero.
pub const DEFAULT_COLOR: Combine = Combine::new(Fn::ScaleOther, F::One, L::Iterated, O::Iterated, false);
pub const DEFAULT_ALPHA: Combine = Combine::new(Fn::ScaleOther, F::One, L::Constant, O::Constant, false);
pub const DEFAULT_TEX: TexCombine = TexCombine::new(Fn::Zero, F::Zero, Fn::Zero, F::Zero, false, false);

/// Expansion of `guColorCombineFunction`. The `Delta0` variants are equal to their
/// plain counterparts except that the hardware ignores the color gradients;
/// returns whether that flat-color mode is on.
pub fn gu_color_combine(fnc: ColorCombineFnc) -> (Combine, bool) {
    use ColorCombineFnc as C;
    let c = |function, factor, local, other| Combine::new(function, factor, local, other, false);
    let combine = match fnc {
        C::Zero => c(Fn::Zero, F::Zero, L::Constant, O::Constant),
        C::Ccrgb => c(Fn::Local, F::Zero, L::Constant, O::Constant),
        C::Itrgb | C::ItrgbDelta0 => c(Fn::Local, F::Zero, L::Iterated, O::Constant),
        C::DecalTexture => c(Fn::ScaleOther, F::One, L::Constant, O::Texture),
        C::TextureTimesCcrgb => c(Fn::ScaleOther, F::Local, L::Constant, O::Texture),
        C::TextureTimesItrgb | C::TextureTimesItrgbDelta0 => c(Fn::ScaleOther, F::Local, L::Iterated, O::Texture),
        C::TextureTimesItrgbAddAlpha => c(Fn::ScaleOtherAddLocalAlpha, F::Local, L::Iterated, O::Texture),
        C::TextureTimesAlpha => c(Fn::ScaleOther, F::LocalAlpha, L::Constant, O::Texture),
        C::TextureTimesAlphaAddItrgb => c(Fn::ScaleOtherAddLocal, F::LocalAlpha, L::Iterated, O::Texture),
        C::TextureAddItrgb => c(Fn::ScaleOtherAddLocal, F::One, L::Iterated, O::Texture),
        C::TextureSubItrgb => c(Fn::ScaleOtherMinusLocal, F::One, L::Iterated, O::Texture),
        C::CcrgbBlendItrgbOnTexalpha => c(Fn::ScaleOtherMinusLocalAddLocal, F::TextureAlpha, L::Constant, O::Iterated),
        C::DiffSpecA => c(Fn::ScaleOtherAddLocal, F::LocalAlpha, L::Iterated, O::Texture),
        C::DiffSpecB => c(Fn::ScaleOtherAddLocalAlpha, F::Local, L::Iterated, O::Texture),
        C::One => Combine::new(Fn::Zero, F::Zero, L::Constant, O::Constant, true),
    };
    (combine, matches!(fnc, C::ItrgbDelta0 | C::TextureTimesItrgbDelta0))
}

/// Expansion of `guAlphaSource`.
pub fn gu_alpha_source(source: AlphaSource) -> Combine {
    let c = |function, factor, local, other| Combine::new(function, factor, local, other, false);
    match source {
        AlphaSource::ConstantAlpha => c(Fn::Local, F::Zero, L::Constant, O::Constant),
        AlphaSource::IteratedAlpha => c(Fn::Local, F::Zero, L::Iterated, O::Constant),
        AlphaSource::TextureAlpha => c(Fn::ScaleOther, F::One, L::Constant, O::Texture),
        AlphaSource::TextureAlphaTimesIteratedAlpha => c(Fn::ScaleOther, F::Local, L::Iterated, O::Texture),
    }
}

/// Expansion of `guTexCombineFunction` / `grTexCombineFunction`.
pub fn gu_tex_combine(fnc: TextureCombineFnc) -> TexCombine {
    use TextureCombineFnc as T;
    let same = |function, factor| TexCombine::new(function, factor, function, factor, false, false);
    match fnc {
        T::Zero => same(Fn::Zero, F::Zero),
        T::Decal => same(Fn::Local, F::Zero),
        T::One => TexCombine::new(Fn::Zero, F::Zero, Fn::Zero, F::Zero, true, true),
        T::Add => same(Fn::ScaleOtherAddLocal, F::One),
        T::Multiply => same(Fn::ScaleOther, F::Local),
        T::Detail => same(Fn::ScaleOtherMinusLocalAddLocal, F::OneMinusTextureAlpha),
        T::DetailOther => same(Fn::ScaleOtherMinusLocalAddLocal, F::TextureAlpha),
        T::TrilinearOdd => same(Fn::ScaleOtherMinusLocalAddLocal, F::OneMinusLodFraction),
        T::TrilinearEven => same(Fn::ScaleOtherMinusLocalAddLocal, F::TextureRgb),
        T::Subtract => same(Fn::ScaleOtherMinusLocal, F::One),
        T::Other => same(Fn::ScaleOther, F::One),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_round_trips_fields() {
        let c = Combine::new(Fn::ScaleMinusLocalAddLocalAlpha, F::OneMinusLodFraction, L::Depth, O::Constant, true);
        let p = c.pack();
        assert_eq!(p & 0x1f, 0x10);
        assert_eq!((p >> 5) & 0xf, 0xd);
        assert_eq!((p >> 9) & 3, 2);
        assert_eq!((p >> 11) & 3, 2);
        assert_eq!(p >> 13, 1);
    }

    #[test]
    fn texture_use() {
        assert!(gu_color_combine(ColorCombineFnc::DecalTexture).0.uses_texture());
        assert!(!gu_color_combine(ColorCombineFnc::Itrgb).0.uses_texture());
        assert!(gu_tex_combine(TextureCombineFnc::Decal).uses_local());
        assert!(!gu_tex_combine(TextureCombineFnc::Decal).uses_other());
        assert!(gu_tex_combine(TextureCombineFnc::Other).uses_other());
        assert!(!gu_tex_combine(TextureCombineFnc::Other).uses_local());
    }
}
