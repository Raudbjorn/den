use ratatui::style::{Color, Modifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalPalette {
    pub bg: Color,
    pub surface_1: Color,
    pub surface_2: Color,
    pub surface_3: Color,
    pub border: Color,
    pub text: Color,
    pub text_strong: Color,
    pub text_muted: Color,
    pub text_faint: Color,
    pub accent: Color,
    pub accent_2: Color,
    pub accent_rust: Color,
    pub mix_target: Color,
    pub success: Color,
    pub error: Color,
    pub warning: Color,
    pub info: Color,
    pub syn_keyword: Color,
    pub syn_string: Color,
    pub syn_var: Color,
    pub syn_func: Color,
    pub syn_comment: Color,
    pub syn_number: Color,
    // den patch: IRONMAW roles with no upstream slot. Real slots, so they are
    // substituted in the same pass as the others, before profile quantisation.
    /// The quiet fill behind a selected row (paired with the rust tick).
    pub selected: Color,
    /// The dark plate behind the bear mark.
    pub mark_bg: Color,
    /// Strong structure lines (slate).
    pub border_strong: Color,
    /// The one action, hovered or focused (lighter rust).
    pub action_hover: Color,
}

/// Sentinel colours that templates write (`text-[#aa0017]`, `bg-[#aa0017]`) and
/// [`crate::component::ComponentRenderer`] replaces with the palette role.
///
/// den patch: named constants for the upstream sentinels, plus `#aa0017..#aa001a`
/// for the IRONMAW roles.
pub mod sentinel {
    use ratatui::style::Color;

    const fn marker(index: u8) -> Color {
        Color::Rgb(170, 0, index)
    }

    pub const BG: Color = marker(0);
    pub const SURFACE_1: Color = marker(1);
    pub const SURFACE_2: Color = marker(2);
    pub const SURFACE_3: Color = marker(3);
    pub const BORDER: Color = marker(4);
    pub const TEXT: Color = marker(5);
    pub const TEXT_STRONG: Color = marker(6);
    pub const TEXT_MUTED: Color = marker(7);
    pub const TEXT_FAINT: Color = marker(8);
    pub const ACCENT: Color = marker(9);
    pub const ACCENT_2: Color = marker(10);
    pub const ACCENT_RUST: Color = marker(11);
    pub const MIX_TARGET: Color = marker(12);
    pub const SUCCESS: Color = marker(13);
    pub const ERROR: Color = marker(14);
    pub const WARNING: Color = marker(15);
    pub const SYN_KEYWORD: Color = marker(16);
    pub const SYN_STRING: Color = marker(17);
    pub const SYN_VAR: Color = marker(18);
    pub const SYN_FUNC: Color = marker(19);
    pub const SYN_COMMENT: Color = marker(20);
    pub const SYN_NUMBER: Color = marker(21);
    pub const INFO: Color = marker(22);
    /// `#aa0017`
    pub const SELECTED: Color = marker(23);
    /// `#aa0018`
    pub const MARK_BG: Color = marker(24);
    /// `#aa0019`
    pub const BORDER_STRONG: Color = marker(25);
    /// `#aa001a`
    pub const ACTION_HOVER: Color = marker(26);
}

impl TerminalPalette {
    fn sentinel_to_role(&self, rgb: (u8, u8, u8)) -> Option<Color> {
        match rgb {
            (170, 0, 0) => Some(self.bg),
            (170, 0, 1) => Some(self.surface_1),
            (170, 0, 2) => Some(self.surface_2),
            (170, 0, 3) => Some(self.surface_3),
            (170, 0, 4) => Some(self.border),
            (170, 0, 5) => Some(self.text),
            (170, 0, 6) => Some(self.text_strong),
            (170, 0, 7) => Some(self.text_muted),
            (170, 0, 8) => Some(self.text_faint),
            (170, 0, 9) => Some(self.accent),
            (170, 0, 10) => Some(self.accent_2),
            (170, 0, 11) => Some(self.accent_rust),
            (170, 0, 12) => Some(self.mix_target),
            (170, 0, 13) => Some(self.success),
            (170, 0, 14) => Some(self.error),
            (170, 0, 15) => Some(self.warning),
            (170, 0, 16) => Some(self.syn_keyword),
            (170, 0, 17) => Some(self.syn_string),
            (170, 0, 18) => Some(self.syn_var),
            (170, 0, 19) => Some(self.syn_func),
            (170, 0, 20) => Some(self.syn_comment),
            (170, 0, 21) => Some(self.syn_number),
            (170, 0, 22) => Some(self.info),
            (170, 0, 23) => Some(self.selected),
            (170, 0, 24) => Some(self.mark_bg),
            (170, 0, 25) => Some(self.border_strong),
            (170, 0, 26) => Some(self.action_hover),
            _ => None,
        }
    }

    pub(crate) fn get_semantic_color(&self, marker: Color) -> Color {
        match marker {
            Color::Rgb(r, g, b) => self.sentinel_to_role((r, g, b)).unwrap_or(marker),
            _ => marker,
        }
    }
}

/// den patch: the text attributes that keep an IRONMAW role readable when the
/// profile has no colour at all (mono, no-color). Applied to a cell whose
/// foreground or background is the role's sentinel. Upstream roles have no cue
/// here; den's templates carry those meanings with glyphs.
pub(crate) fn mono_cue(marker: Color) -> Modifier {
    match marker {
        sentinel::SELECTED => Modifier::REVERSED,
        sentinel::MARK_BG => Modifier::DIM,
        sentinel::BORDER_STRONG => Modifier::BOLD,
        sentinel::ACTION_HOVER => Modifier::BOLD.union(Modifier::UNDERLINED),
        _ => Modifier::empty(),
    }
}
