//! APCA 对比度调整：ANSI 颜色在单元格背景上的可读性下限。
//!
//! 浅色主题下的亮色（亮白、亮黄）与深色主题下的暗色常常和背景近乎
//! 同色，直接绘制会“看不见”。这里按 APCA（常数取 0.0.98G-4g，
//! <https://github.com/Myndex/apca-w3>）计算前景与背景的感知对比度，
//! 低于阈值时先只调整明度，其次降低饱和度再调明度，最后才退回黑 / 白，
//! 全程保留 alpha。行为与 Zed 的 `ensure_minimum_contrast` 一致。
//!
//! 调用方负责跳过不该调整的颜色与字符：TUI 明确指定的 256 色（索引
//! ≥ 16）与真彩色，以及盒线、块元素这类装饰字符。

use gpui_kit::Hsla;

/// 终端正文的最小对比度（APCA Lc）；与 Zed 的终端默认值一致。
pub const TERMINAL_MINIMUM_CONTRAST: f32 = 45.;

// APCA 0.0.98G-4g 常数。
const MAIN_TRC: f32 = 2.4;
const S_RCO: f32 = 0.2126729;
const S_GCO: f32 = 0.7151522;
const S_BCO: f32 = 0.0721750;
const NORM_BG: f32 = 0.56;
const NORM_TXT: f32 = 0.57;
const REV_TXT: f32 = 0.62;
const REV_BG: f32 = 0.65;
const BLK_THRS: f32 = 0.022;
const BLK_CLMP: f32 = 1.414;
const SCALE_BOW: f32 = 1.14;
const SCALE_WOB: f32 = 1.14;
const LO_BOW_OFFSET: f32 = 0.027;
const LO_WOB_OFFSET: f32 = 0.027;
const DELTA_Y_MIN: f32 = 0.0005;
const LO_CLIP: f32 = 0.1;

/// APCA 感知亮度 Y：sRGB 各通道取幂后按系数加权。
fn luminance(color: Hsla) -> f32 {
    let rgba = color.to_rgb();
    S_RCO * rgba.r.powf(MAIN_TRC) + S_GCO * rgba.g.powf(MAIN_TRC) + S_BCO * rgba.b.powf(MAIN_TRC)
}

/// APCA 对比度 Lc，约 -108 到 106；负值表示浅色文字在深色背景上。
pub fn apca_contrast(text: Hsla, background: Hsla) -> f32 {
    let text_y = luminance(text);
    let background_y = luminance(background);

    // 近黑颜色软钳制，避免极暗时对比度塌陷。
    let clamp_black = |y: f32| {
        if y > BLK_THRS {
            y
        } else {
            y + (BLK_THRS - y).powf(BLK_CLMP)
        }
    };
    let text_y = clamp_black(text_y);
    let background_y = clamp_black(background_y);

    if (background_y - text_y).abs() < DELTA_Y_MIN {
        return 0.;
    }

    if background_y > text_y {
        // 深色文字在浅色背景上。
        let sapc = (background_y.powf(NORM_BG) - text_y.powf(NORM_TXT)) * SCALE_BOW;
        if sapc < LO_CLIP {
            0.
        } else {
            (sapc - LO_BOW_OFFSET) * 100.
        }
    } else {
        // 浅色文字在深色背景上。
        let sapc = (background_y.powf(REV_BG) - text_y.powf(REV_TXT)) * SCALE_WOB;
        if sapc > -LO_CLIP {
            0.
        } else {
            (sapc + LO_WOB_OFFSET) * 100.
        }
    }
}

/// 让前景色与背景达到最小对比度；已达标时原样返回。
pub fn ensure_minimum_contrast(foreground: Hsla, background: Hsla, minimum: f32) -> Hsla {
    if minimum <= 0. || apca_contrast(foreground, background).abs() >= minimum {
        return foreground;
    }

    let adjusted = adjust_lightness(foreground, background, minimum);
    if apca_contrast(adjusted, background).abs() >= minimum {
        return adjusted;
    }

    let desaturated = adjust_saturation_and_lightness(foreground, background, minimum);
    if apca_contrast(desaturated, background).abs() >= minimum {
        return desaturated;
    }

    // 纯黑 / 纯白兜底，保留 alpha。
    let black = Hsla {
        h: 0.,
        s: 0.,
        l: 0.,
        a: foreground.a,
    };
    let white = Hsla {
        h: 0.,
        s: 0.,
        l: 1.,
        a: foreground.a,
    };
    if apca_contrast(white, background).abs() > apca_contrast(black, background).abs() {
        white
    } else {
        black
    }
}

/// 二分查找刚好达标的明度，保持色相与饱和度。
fn adjust_lightness(foreground: Hsla, background: Hsla, minimum: f32) -> Hsla {
    // 浅色背景向更暗调整，深色背景向更亮调整。
    let go_darker = luminance(background) > 0.5;
    let (mut low, mut high) = if go_darker {
        (0., foreground.l)
    } else {
        (foreground.l, 1.)
    };
    let mut best = foreground.l;

    for _ in 0..20 {
        let mid = (low + high) / 2.;
        let candidate = Hsla {
            l: mid,
            ..foreground
        };
        let contrast = apca_contrast(candidate, background).abs();

        if contrast >= minimum {
            best = mid;
            if go_darker {
                low = mid;
            } else {
                high = mid;
            }
        } else if go_darker {
            high = mid;
        } else {
            low = mid;
        }

        if (contrast - minimum).abs() < 1. {
            best = mid;
            break;
        }
    }

    Hsla {
        l: best,
        ..foreground
    }
}

/// 依次降低饱和度并调整明度；仍不达标时返回去饱和结果。
fn adjust_saturation_and_lightness(foreground: Hsla, background: Hsla, minimum: f32) -> Hsla {
    for multiplier in [1.0, 0.8, 0.6, 0.4, 0.2, 0.0] {
        let candidate = Hsla {
            s: foreground.s * multiplier,
            ..foreground
        };
        let adjusted = adjust_lightness(candidate, background, minimum);
        if apca_contrast(adjusted, background).abs() >= minimum {
            return adjusted;
        }
    }

    Hsla {
        s: 0.,
        ..foreground
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::rgb;

    fn hex(value: u32) -> Hsla {
        rgb(value).into()
    }

    /// 黑字白底为正向高对比，白字黑底为负向高对比，同色接近 0。
    #[test]
    fn contrast_is_polarity_aware() {
        let black = hex(0x000000);
        let white = hex(0xffffff);

        assert!(apca_contrast(black, white) > 100.);
        assert!(apca_contrast(white, black) < -100.);
        assert!(apca_contrast(hex(0x808080), hex(0x808080)).abs() < 1.);

        let dark_on_light = apca_contrast(black, white);
        let light_on_dark = apca_contrast(white, black);
        assert!((dark_on_light + light_on_dark).abs() > 1.);
    }

    /// 已达标的颜色原样返回；alpha 不变。
    #[test]
    fn readable_colors_are_untouched() {
        let foreground = hex(0x30352a);
        let background = hex(0xf4f5ef);
        assert_eq!(
            ensure_minimum_contrast(foreground, background, TERMINAL_MINIMUM_CONTRAST),
            foreground
        );
        assert_eq!(
            ensure_minimum_contrast(foreground, background, 0.),
            foreground,
            "门槛为 0 时不做任何调整"
        );
    }

    /// 与背景同色的前景会被调整到最小对比度，且保留 alpha。
    #[test]
    fn invisible_colors_are_adjusted() {
        let background = hex(0xf4f5ef);

        let light = Hsla {
            a: 0.8,
            ..hex(0xf4f5ef)
        };
        let adjusted = ensure_minimum_contrast(light, background, TERMINAL_MINIMUM_CONTRAST);
        assert!(apca_contrast(adjusted, background).abs() >= TERMINAL_MINIMUM_CONTRAST);
        assert_eq!(adjusted.a, 0.8);

        let dark = hex(0x2a2c28);
        let on_dark = Hsla {
            a: 1.,
            ..hex(0x101210)
        };
        let adjusted = ensure_minimum_contrast(on_dark, dark, TERMINAL_MINIMUM_CONTRAST);
        assert!(apca_contrast(adjusted, dark).abs() >= TERMINAL_MINIMUM_CONTRAST);
    }

    /// 亮色在浅色背景上被压暗，暗色在深色背景上被提亮。
    #[test]
    fn adjustment_follows_background_lightness() {
        let bright_green = hex(0x5f8a45);
        let adjusted =
            ensure_minimum_contrast(bright_green, hex(0xf4f5ef), TERMINAL_MINIMUM_CONTRAST);
        assert!(adjusted.l <= bright_green.l, "浅色背景应压暗");

        let dark_green = hex(0x2f4a25);
        let adjusted =
            ensure_minimum_contrast(dark_green, hex(0x14181a), TERMINAL_MINIMUM_CONTRAST);
        assert!(adjusted.l >= dark_green.l, "深色背景应提亮");
    }
}
