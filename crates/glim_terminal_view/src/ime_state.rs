//! 输入法预编辑状态：纯视图状态，不依赖 UI 框架。

/// 输入法组合（预编辑）状态。
#[derive(Default)]
pub(crate) struct ImeState {
    /// 当前预编辑（marked）文本；`None` 表示没有组合。
    pub preedit: Option<String>,
}

impl ImeState {
    /// 预编辑文本的 UTF-16 长度（范围以 UTF-16 代码单元计）。
    pub fn preedit_len_utf16(&self) -> usize {
        self.preedit
            .as_ref()
            .map(|text| text.encode_utf16().count())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preedit_len_counts_utf16_units() {
        let mut state = ImeState::default();
        assert_eq!(state.preedit_len_utf16(), 0);
        state.preedit = Some("中a".to_owned());
        assert_eq!(state.preedit_len_utf16(), 2);
        state.preedit = Some("𠀀".to_owned());
        assert_eq!(state.preedit_len_utf16(), 2);
    }
}
