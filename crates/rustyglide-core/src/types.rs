//! Glide 2 enumerations and constants as typed Rust values.
//!
//! The numeric values are interface facts of the Glide 2.4x ABI. Every enum
//! converts from the raw `u32` a C caller passes with `TryFrom`; an unknown
//! value is an error the front end reports, never a silent default.

/// Error for a raw value that names no member of a Glide enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidValue {
    pub kind: &'static str,
    pub value: u32,
}

impl std::fmt::Display for InvalidValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid {} value {:#x}", self.kind, self.value)
    }
}

impl std::error::Error for InvalidValue {}

macro_rules! glide_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $value:expr),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
        #[repr(u32)]
        pub enum $name {
            #[default]
            $($(#[$vmeta])* $variant = $value),+
        }

        impl TryFrom<u32> for $name {
            type Error = InvalidValue;
            fn try_from(value: u32) -> Result<Self, InvalidValue> {
                match value {
                    $(v if v == $value => Ok(Self::$variant),)+
                    _ => Err(InvalidValue { kind: stringify!($name), value }),
                }
            }
        }

        impl From<$name> for u32 {
            fn from(value: $name) -> u32 {
                value as u32
            }
        }
    };
}

glide_enum!(
    /// A texture mapping unit.
    Tmu { Tmu0 = 0, Tmu1 = 1, Tmu2 = 2 }
);

impl Tmu {
    pub fn index(self) -> usize {
        self as usize
    }
}

glide_enum!(
    /// `GrCombineFunction_t`.
    CombineFunction {
        Zero = 0x0,
        Local = 0x1,
        LocalAlpha = 0x2,
        ScaleOther = 0x3,
        ScaleOtherAddLocal = 0x4,
        ScaleOtherAddLocalAlpha = 0x5,
        ScaleOtherMinusLocal = 0x6,
        ScaleOtherMinusLocalAddLocal = 0x7,
        ScaleOtherMinusLocalAddLocalAlpha = 0x8,
        ScaleMinusLocalAddLocal = 0x9,
        ScaleMinusLocalAddLocalAlpha = 0x10,
    }
);

glide_enum!(
    /// `GrCombineFactor_t`. Bit 3 selects "one minus" the base factor in bits 0-2.
    CombineFactor {
        Zero = 0x0,
        Local = 0x1,
        OtherAlpha = 0x2,
        LocalAlpha = 0x3,
        /// Texture alpha in the FBI combiners, the detail factor in a TMU.
        TextureAlpha = 0x4,
        /// Texture RGB in the FBI combiners, the LOD fraction in a TMU.
        TextureRgb = 0x5,
        One = 0x8,
        OneMinusLocal = 0x9,
        OneMinusOtherAlpha = 0xa,
        OneMinusLocalAlpha = 0xb,
        OneMinusTextureAlpha = 0xc,
        OneMinusLodFraction = 0xd,
    }
);

glide_enum!(
    /// `GrCombineLocal_t`.
    CombineLocal { Iterated = 0x0, Constant = 0x1, Depth = 0x2 }
);

glide_enum!(
    /// `GrCombineOther_t`.
    CombineOther { Iterated = 0x0, Texture = 0x1, Constant = 0x2 }
);

glide_enum!(
    /// `GrAlphaSource_t` for `guAlphaSource`.
    AlphaSource {
        ConstantAlpha = 0x0,
        IteratedAlpha = 0x1,
        TextureAlpha = 0x2,
        TextureAlphaTimesIteratedAlpha = 0x3,
    }
);

glide_enum!(
    /// `GrColorCombineFnc_t` for `guColorCombineFunction`.
    ColorCombineFnc {
        Zero = 0x0,
        Ccrgb = 0x1,
        Itrgb = 0x2,
        ItrgbDelta0 = 0x3,
        DecalTexture = 0x4,
        TextureTimesCcrgb = 0x5,
        TextureTimesItrgb = 0x6,
        TextureTimesItrgbDelta0 = 0x7,
        TextureTimesItrgbAddAlpha = 0x8,
        TextureTimesAlpha = 0x9,
        TextureTimesAlphaAddItrgb = 0xa,
        TextureAddItrgb = 0xb,
        TextureSubItrgb = 0xc,
        CcrgbBlendItrgbOnTexalpha = 0xd,
        DiffSpecA = 0xe,
        DiffSpecB = 0xf,
        One = 0x10,
    }
);

glide_enum!(
    /// `GrAlphaBlendFnc_t`. Values 2 and 6 mean the other side's color, and 0xf
    /// is alpha-saturate as a source factor and pre-fog color as a destination one.
    BlendFactor {
        Zero = 0x0,
        SrcAlpha = 0x1,
        Color = 0x2,
        DstAlpha = 0x3,
        One = 0x4,
        OneMinusSrcAlpha = 0x5,
        OneMinusColor = 0x6,
        OneMinusDstAlpha = 0x7,
        AlphaSaturateOrPrefog = 0xf,
    }
);

glide_enum!(
    /// `GrAspectRatio_t` in Glide 2 numbering (8x1 is 0, 1x8 is 6).
    Aspect {
        A8x1 = 0x0,
        A4x1 = 0x1,
        A2x1 = 0x2,
        A1x1 = 0x3,
        A1x2 = 0x4,
        A1x4 = 0x5,
        A1x8 = 0x6,
    }
);

glide_enum!(
    /// `GrBuffer_t`.
    Buffer {
        Front = 0x0,
        Back = 0x1,
        Aux = 0x2,
        Depth = 0x3,
        Alpha = 0x4,
        Triple = 0x5,
    }
);

glide_enum!(
    /// `GrCmpFnc_t`.
    CmpFnc {
        Never = 0x0,
        Less = 0x1,
        Equal = 0x2,
        LEqual = 0x3,
        Greater = 0x4,
        NotEqual = 0x5,
        GEqual = 0x6,
        Always = 0x7,
    }
);

impl CmpFnc {
    /// Evaluates `new <op> old` on integer values, as the hardware compares.
    pub fn test(self, new: u32, old: u32) -> bool {
        match self {
            Self::Never => false,
            Self::Less => new < old,
            Self::Equal => new == old,
            Self::LEqual => new <= old,
            Self::Greater => new > old,
            Self::NotEqual => new != old,
            Self::GEqual => new >= old,
            Self::Always => true,
        }
    }
}

glide_enum!(
    /// `GrColorFormat_t`: byte order of a packed `GrColor_t`.
    ColorFormat { Argb = 0x0, Abgr = 0x1, Rgba = 0x2, Bgra = 0x3 }
);

impl ColorFormat {
    /// Unpacks a `GrColor_t` into `[r, g, b, a]`.
    pub fn unpack(self, c: u32) -> [u8; 4] {
        let b = c.to_le_bytes();
        match self {
            Self::Argb => [b[2], b[1], b[0], b[3]],
            Self::Abgr => [b[0], b[1], b[2], b[3]],
            Self::Rgba => [b[3], b[2], b[1], b[0]],
            Self::Bgra => [b[1], b[2], b[3], b[0]],
        }
    }
}

glide_enum!(
    /// `GrCullMode_t`.
    CullMode { Disable = 0x0, Negative = 0x1, Positive = 0x2 }
);

glide_enum!(
    /// `GrDepthBufferMode_t`.
    DepthBufferMode {
        Disable = 0x0,
        ZBuffer = 0x1,
        WBuffer = 0x2,
        ZBufferCompareToBias = 0x3,
        WBufferCompareToBias = 0x4,
    }
);

glide_enum!(
    /// `GrDitherMode_t`.
    DitherMode { Disable = 0x0, D2x2 = 0x1, D4x4 = 0x2 }
);

/// `GrFogMode_t`: a base mode plus the `MULT2`/`ADD2` two-pass flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FogMode(pub u32);

impl FogMode {
    pub const DISABLE: u32 = 0x0;
    pub const WITH_ITERATED_ALPHA: u32 = 0x1;
    pub const WITH_TABLE: u32 = 0x2;
    pub const WITH_ITERATED_Z: u32 = 0x3;
    pub const MULT2: u32 = 0x100;
    pub const ADD2: u32 = 0x200;

    pub fn base(self) -> u32 {
        self.0 & 0xff
    }
}

impl TryFrom<u32> for FogMode {
    type Error = InvalidValue;
    fn try_from(value: u32) -> Result<Self, InvalidValue> {
        if value & !0x3ff != 0 || value & 0xff > 3 {
            return Err(InvalidValue { kind: "FogMode", value });
        }
        Ok(Self(value))
    }
}

glide_enum!(
    /// `GrLfbWriteMode_t`.
    LfbWriteMode {
        W565 = 0x0,
        W555 = 0x1,
        W1555 = 0x2,
        W888 = 0x4,
        W8888 = 0x5,
        W565Depth = 0xc,
        W555Depth = 0xd,
        W1555Depth = 0xe,
        Za16 = 0xf,
        Any = 0xff,
    }
);

impl LfbWriteMode {
    /// Bytes per pixel in the locked buffer.
    pub fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::W888 | Self::W8888 | Self::W565Depth | Self::W555Depth | Self::W1555Depth => 4,
            _ => 2,
        }
    }
}

glide_enum!(
    /// `GrOriginLocation_t`.
    Origin { UpperLeft = 0x0, LowerLeft = 0x1, Any = 0xff }
);

glide_enum!(
    /// `GrLOD_t` in Glide 2 numbering: 0 is 256 texels, 8 is 1 texel.
    Lod {
        L256 = 0x0,
        L128 = 0x1,
        L64 = 0x2,
        L32 = 0x3,
        L16 = 0x4,
        L8 = 0x5,
        L4 = 0x6,
        L2 = 0x7,
        L1 = 0x8,
    }
);

impl Lod {
    /// Edge length of the long side in texels.
    pub fn size(self) -> u32 {
        256 >> (self as u32)
    }

    pub fn from_index(index: u32) -> Option<Self> {
        Self::try_from(index).ok()
    }
}

glide_enum!(
    /// `GrMipMapMode_t`.
    MipMapMode { Disable = 0x0, Nearest = 0x1, NearestDither = 0x2 }
);

glide_enum!(
    /// `GrTextureClampMode_t`.
    ClampMode { Wrap = 0x0, Clamp = 0x1 }
);

glide_enum!(
    /// `GrTextureCombineFnc_t` for `guTexCombineFunction`.
    TextureCombineFnc {
        Zero = 0x0,
        Decal = 0x1,
        Other = 0x2,
        Add = 0x3,
        Multiply = 0x4,
        Subtract = 0x5,
        Detail = 0x6,
        DetailOther = 0x7,
        TrilinearOdd = 0x8,
        TrilinearEven = 0x9,
        One = 0xa,
    }
);

glide_enum!(
    /// `GrTextureFilterMode_t`.
    FilterMode { PointSampled = 0x0, Bilinear = 0x1 }
);

glide_enum!(
    /// `GrTextureFormat_t`.
    TextureFormat {
        Rgb332 = 0x0,
        Yiq422 = 0x1,
        Alpha8 = 0x2,
        Intensity8 = 0x3,
        AlphaIntensity44 = 0x4,
        P8 = 0x5,
        Argb8332 = 0x8,
        Ayiq8422 = 0x9,
        Rgb565 = 0xa,
        Argb1555 = 0xb,
        Argb4444 = 0xc,
        AlphaIntensity88 = 0xd,
        Ap88 = 0xe,
    }
);

impl TextureFormat {
    pub fn bytes_per_texel(self) -> u32 {
        if (self as u32) >= 0x8 { 2 } else { 1 }
    }

    pub fn uses_palette(self) -> bool {
        matches!(self, Self::P8 | Self::Ap88)
    }

    pub fn uses_ncc(self) -> bool {
        matches!(self, Self::Yiq422 | Self::Ayiq8422)
    }
}

glide_enum!(
    /// `GrTexTable_t`.
    TexTable { Ncc0 = 0x0, Ncc1 = 0x1, Palette = 0x2 }
);

glide_enum!(
    /// `GrNCCTable_t`.
    NccTable { Ncc0 = 0x0, Ncc1 = 0x1 }
);

glide_enum!(
    /// `GrTexBaseRange_t` for multibase textures.
    TexBaseRange { B256 = 0x0, B128 = 0x1, B64 = 0x2, B32To1 = 0x3 }
);

glide_enum!(
    /// `GrChromakeyMode_t`.
    ChromakeyMode { Disable = 0x0, Enable = 0x1 }
);

glide_enum!(
    /// `GrHint_t`.
    Hint {
        StwHint = 0,
        FifoCheckHint = 1,
        FpuPrecision = 2,
        AllowMipMapDither = 3,
        LfbWrite = 4,
        LfbProtect = 5,
        LfbReset = 6,
    }
);

/// `GrSTWHint_t` bits: which per-TMU vertex fields differ from the shared ones.
pub mod stw_hint {
    pub const W_DIFF_FBI: u32 = 1 << 0;
    pub const W_DIFF_TMU0: u32 = 1 << 1;
    pub const ST_DIFF_TMU0: u32 = 1 << 2;
    pub const W_DIFF_TMU1: u32 = 1 << 3;
    pub const ST_DIFF_TMU1: u32 = 1 << 4;
    pub const W_DIFF_TMU2: u32 = 1 << 5;
    pub const ST_DIFF_TMU2: u32 = 1 << 6;
}

/// `GrMipMapLevelMask` bits for `evenOdd` arguments.
pub mod level_mask {
    pub const EVEN: u32 = 1;
    pub const ODD: u32 = 2;
    pub const BOTH: u32 = 3;
}

glide_enum!(
    /// `GrScreenResolution_t`.
    Resolution {
        R320x200 = 0x0,
        R320x240 = 0x1,
        R400x256 = 0x2,
        R512x384 = 0x3,
        R640x200 = 0x4,
        R640x350 = 0x5,
        R640x400 = 0x6,
        R640x480 = 0x7,
        R800x600 = 0x8,
        R960x720 = 0x9,
        R856x480 = 0xa,
        R512x256 = 0xb,
        R1024x768 = 0xc,
        R1280x1024 = 0xd,
        R1600x1200 = 0xe,
        R400x300 = 0xf,
        None = 0xff,
    }
);

impl Resolution {
    /// Size in pixels, or `None` for `GR_RESOLUTION_NONE` (use the window's size).
    pub fn size(self) -> Option<(u32, u32)> {
        Some(match self {
            Self::R320x200 => (320, 200),
            Self::R320x240 => (320, 240),
            Self::R400x256 => (400, 256),
            Self::R512x384 => (512, 384),
            Self::R640x200 => (640, 200),
            Self::R640x350 => (640, 350),
            Self::R640x400 => (640, 400),
            Self::R640x480 => (640, 480),
            Self::R800x600 => (800, 600),
            Self::R960x720 => (960, 720),
            Self::R856x480 => (856, 480),
            Self::R512x256 => (512, 256),
            Self::R1024x768 => (1024, 768),
            Self::R1280x1024 => (1280, 1024),
            Self::R1600x1200 => (1600, 1200),
            Self::R400x300 => (400, 300),
            Self::None => return None,
        })
    }
}

/// Lock type bits for `grLfbLock`.
pub mod lock {
    pub const READ_ONLY: u32 = 0x00;
    pub const WRITE_ONLY: u32 = 0x01;
    pub const NO_IDLE: u32 = 0x10;
}

/// `GrLfbSrcFmt_t` for `grLfbWriteRegion`.
pub mod lfb_src {
    pub const RLE16: u32 = 0x80;
}

/// Number of entries in a Glide 2 fog table.
pub const FOG_TABLE_SIZE: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_reject_unknown_values() {
        assert_eq!(CombineFunction::try_from(0x10), Ok(CombineFunction::ScaleMinusLocalAddLocalAlpha));
        assert!(CombineFunction::try_from(0xa).is_err());
        assert_eq!(Lod::L32.size(), 32);
        assert!(FogMode::try_from(FogMode::WITH_TABLE | FogMode::ADD2).is_ok());
        assert!(FogMode::try_from(4).is_err());
    }

    #[test]
    fn color_formats_unpack_to_rgba() {
        assert_eq!(ColorFormat::Argb.unpack(0x80112233), [0x11, 0x22, 0x33, 0x80]);
        assert_eq!(ColorFormat::Abgr.unpack(0x80332211), [0x11, 0x22, 0x33, 0x80]);
        assert_eq!(ColorFormat::Rgba.unpack(0x11223380), [0x11, 0x22, 0x33, 0x80]);
        assert_eq!(ColorFormat::Bgra.unpack(0x33221180), [0x11, 0x22, 0x33, 0x80]);
    }
}
