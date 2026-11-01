//! CONTRACT: colour theme.
//!
//! Loaded from `~/.config/tidings/colors.json` (written by the Noctalia hook
//! script). Keys are camelCase versions of the fields; colours are `#rrggbb`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub mode: Mode,
    pub surface: Rgb,
    pub surface_variant: Rgb,
    pub on_surface: Rgb,
    pub on_surface_variant: Rgb,
    pub primary: Rgb,
    pub on_primary: Rgb,
    pub error: Rgb,
    pub on_error: Rgb,
    pub outline: Rgb,
    /// Panel/toast background alpha (glass). 0.0..=1.0
    pub surface_alpha: f32,
}
