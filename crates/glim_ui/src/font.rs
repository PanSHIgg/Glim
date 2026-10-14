//! 终端字体：产品默认字族的解析与应用。
//!
//! GPUI 的文本系统在不存在的字族上会 panic，因此产品默认与用户配置
//! 的字族都在这里对照已安装字体校验后再交给主题。默认字族优先
//! Cascadia Mono，其次 Consolas，再退回平台等宽字体；终端元素另外
//! 提供中文回退链（见 `glim_terminal_view` 的字体构造）。

use gpui_kit::component::Theme;
use gpui_kit::*;

use crate::{ThemePreference, apply_theme_preference};

/// 终端字号的产品范围（逻辑像素）。
pub const MIN_TERMINAL_FONT_SIZE: f32 = 8.;
/// 终端字号上限（逻辑像素）。
pub const MAX_TERMINAL_FONT_SIZE: f32 = 32.;
/// 产品默认终端字号（设计稿终端正文 0.875rem）。
pub const DEFAULT_TERMINAL_FONT_SIZE: f32 = 14.;
/// 默认字族的查找顺序。
const DEFAULT_FAMILIES: &[&str] = &["Cascadia Mono", "Consolas"];

/// 字体应用结果。
#[non_exhaustive]
pub struct FontApplication {
    /// 实际应用的字族。
    pub applied: SharedString,
    /// 被请求但未安装的字族；发生时已回退到默认。
    pub missing: Option<SharedString>,
}

/// 产品默认终端字体：在 [`init`] 时解析一次。
struct TerminalFontDefaults {
    family: SharedString,
    size: Pixels,
}

impl Global for TerminalFontDefaults {}

/// 解析并应用产品默认终端字体；在 `glim_ui` 初始化时调用。
pub(crate) fn init(cx: &mut App) {
    let platform_default = Theme::global(cx).mono_font_family.clone();
    let installed = cx.text_system().all_font_names();
    let family = pick_family(DEFAULT_FAMILIES, &installed).unwrap_or(platform_default);
    Theme::update(cx, |theme| {
        theme.mono_font_family = family.clone();
        theme.mono_font_size = px(DEFAULT_TERMINAL_FONT_SIZE);
    });
    cx.set_global(TerminalFontDefaults {
        family,
        size: px(DEFAULT_TERMINAL_FONT_SIZE),
    });
}

/// 内置默认字族（Cascadia Mono → Consolas → 平台等宽）。
pub fn default_mono_font_family(cx: &App) -> SharedString {
    match cx.try_global::<TerminalFontDefaults>() {
        Some(defaults) => defaults.family.clone(),
        None => Theme::global(cx).mono_font_family.clone(),
    }
}

/// 内置默认字号。
pub fn default_mono_font_size(cx: &App) -> Pixels {
    match cx.try_global::<TerminalFontDefaults>() {
        Some(defaults) => defaults.size,
        None => Theme::global(cx).mono_font_size,
    }
}

/// 应用终端字体设置并即时刷新窗口。
///
/// `family` 为空或未安装时回退到内置默认；`size` 为 `None` 时使用内置
/// 默认字号，越界值收敛到产品范围。
pub fn apply_mono_font(family: Option<&str>, size: Option<f32>, cx: &mut App) -> FontApplication {
    let installed = cx.text_system().all_font_names();
    let default_family = default_mono_font_family(cx);
    let (resolved, missing) = resolve_family(family, &default_family, &installed);
    let resolved_size = px(size
        .unwrap_or(f32::from(default_mono_font_size(cx)))
        .clamp(MIN_TERMINAL_FONT_SIZE, MAX_TERMINAL_FONT_SIZE));
    Theme::update(cx, |theme| {
        theme.mono_font_family = resolved.clone();
        theme.mono_font_size = resolved_size;
    });
    FontApplication {
        applied: resolved,
        missing,
    }
}

/// 应用外观设置：主题偏好与终端字体；返回需要提示的字体回退信息。
pub fn apply_appearance(
    preference: ThemePreference,
    family: Option<&str>,
    size: Option<f32>,
    window: Option<&mut Window>,
    cx: &mut App,
) -> Option<String> {
    apply_theme_preference(preference, window, cx);
    let font = apply_mono_font(family, size, cx);
    font.missing
        .map(|family| format!("字体“{family}”未安装，终端已使用“{}”", font.applied))
}

/// 选择第一个已安装的候选字族（忽略大小写差异）。
fn pick_family(candidates: &[&str], installed: &[String]) -> Option<SharedString> {
    candidates
        .iter()
        .find(|candidate| {
            installed
                .iter()
                .any(|name| name.eq_ignore_ascii_case(candidate))
        })
        .map(|family| SharedString::from(*family))
}

/// 解析请求的字族：已安装则使用，否则回退到默认。
fn resolve_family(
    requested: Option<&str>,
    default: &SharedString,
    installed: &[String],
) -> (SharedString, Option<SharedString>) {
    let Some(requested) = requested.map(str::trim).filter(|name| !name.is_empty()) else {
        return (default.clone(), None);
    };
    if installed
        .iter()
        .any(|name| name.eq_ignore_ascii_case(requested))
    {
        (SharedString::from(requested), None)
    } else {
        (default.clone(), Some(SharedString::from(requested)))
    }
}

#[cfg(test)]
mod tests {
    use super::{pick_family, resolve_family};
    use gpui_kit::SharedString;

    fn installed(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn pick_family_prefers_first_installed_candidate() {
        let names = installed(&["Consolas", "Courier New"]);
        assert_eq!(
            pick_family(&["Cascadia Mono", "Consolas"], &names),
            Some("Consolas".into())
        );
        assert_eq!(pick_family(&["Cascadia Mono"], &names), None);
    }

    #[test]
    fn resolve_family_keeps_installed_requests() {
        let default = SharedString::from("Consolas");
        let names = installed(&["JetBrains Mono"]);
        let (family, missing) = resolve_family(Some("JetBrains Mono"), &default, &names);
        assert_eq!(family, SharedString::from("JetBrains Mono"));
        assert!(missing.is_none());
    }

    #[test]
    fn resolve_family_falls_back_for_missing_and_empty() {
        let default = SharedString::from("Consolas");
        let names = installed(&["Consolas"]);
        let (family, missing) = resolve_family(Some("Not A Font"), &default, &names);
        assert_eq!(family, default);
        assert_eq!(missing, Some(SharedString::from("Not A Font")));

        let (family, missing) = resolve_family(Some("   "), &default, &names);
        assert_eq!(family, default);
        assert!(missing.is_none());
    }
}
