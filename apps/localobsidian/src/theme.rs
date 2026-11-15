//! The app's one colour that is not the theme's greys: the violet of the icon, used where the note
//! itself is the content (links, tag chips, the bar of a plain quote). Everything else stays with the
//! desktop theme - this is a note app, not a brand exercise.

/// The icon's violet. The light theme needs it a touch darker to stay readable on white; the dark
/// theme a touch lighter for the same reason. Both are the same hue.
pub const ACCENT_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x5b, 0x52, 0xd6);
pub const ACCENT_DARK: egui::Color32 = egui::Color32::from_rgb(0xa9, 0xa3, 0xff);

/// The accent for the theme in use.
pub fn accent(dark_mode: bool) -> egui::Color32 {
    if dark_mode { ACCENT_DARK } else { ACCENT_LIGHT }
}
