//! View > Theme: choosing a preset or one of the user's saved themes, the
//! Theme Editor (every colour plus the chrome font, live preview, save as
//! "my theme"), and copying / pasting a theme as text.

use nodi_core::storage::{format_hex_color, parse_hex_color, Theme, ThemeSpec};

use crate::app::App;
use crate::wnd::form::{Field, Form};
use crate::wnd::theme::{self, active_spec, presets};
use crate::wnd::{clipboard, dialogs, image, menu, status};

/// Name of the temporary entry used while the Theme Editor previews.
const PREVIEW: &str = "(preview)";

impl App {
    /// View > Background: Plain / Lines / Grid / Dots.
    pub fn set_background(&mut self, background: nodi_core::Background) {
        self.settings.background = background;
        self.save_settings();
        let palette = self.palette;
        crate::wnd::pattern::set(background, palette.pattern, palette.editor_bg);
        for tab in &self.tabs {
            crate::wnd::editor::set_background(tab.editor, palette.editor_bg);
            // SAFETY: repainting our own editor.
            unsafe {
                windows_sys::Win32::Graphics::Gdi::InvalidateRect(tab.editor, std::ptr::null(), 1);
            }
        }
        menu::sync_settings(self);
    }

    /// Switches to the preset or saved theme called `name`.
    pub fn set_theme_name(&mut self, name: &str) {
        self.settings.theme_name = name.to_owned();
        let dark = theme::Palette::from_settings(&self.settings).dark;
        // Older versions only read `theme`.
        self.settings.theme = if dark { Theme::Dark } else { Theme::Light };
        theme::apply(self);
        self.save_settings();
        menu::sync_settings(self);
        menu::refresh_theme_menu(self);
        status::set_message(self, &format!("Theme: {name}"));
    }

    /// View > Theme > Copy Theme as Text: the active theme as JSON, to share
    /// or keep.
    pub fn copy_theme(&mut self) {
        let mut spec = active_spec(&self.settings);
        spec.clamp();
        if clipboard::set_text(self.hwnd, &spec.to_json()) {
            status::set_message(self, &format!("Theme \"{}\" copied as text", spec.name));
        }
    }

    /// View > Theme > Paste Theme from Text: a theme copied before (JSON) is
    /// saved as one of "my themes" and switched to.
    pub fn paste_theme(&mut self) {
        let text = image::clipboard_text(self.hwnd).unwrap_or_default();
        let Some(spec) = ThemeSpec::from_json(&text) else {
            dialogs::info(
                self.hwnd,
                "The clipboard does not hold a Nodi theme.\n\n\
                 Copy one first with View > Theme > Copy Theme as Text.",
            );
            return;
        };
        let name = self.save_my_theme(spec);
        self.set_theme_name(&name);
    }

    /// Stores `spec` among "my themes" (replacing one with the same name; a
    /// preset's name gets " (mine)") and returns the name it was saved under.
    fn save_my_theme(&mut self, mut spec: ThemeSpec) -> String {
        spec.clamp();
        if presets().iter().any(|preset| preset.name == spec.name) || spec.name == PREVIEW {
            spec.name = format!("{} (mine)", spec.name);
        }
        let name = spec.name.clone();
        match self
            .settings
            .my_themes
            .iter_mut()
            .find(|theme| theme.name == name)
        {
            Some(existing) => *existing = spec,
            None => self.settings.my_themes.push(spec),
        }
        self.save_settings();
        name
    }

    /// View > Theme > Theme Editor...: every colour of the window and the
    /// chrome font; Preview shows it live, OK saves it as one of "my themes".
    pub fn edit_theme(&mut self) {
        let previous = self.settings.clone();
        let mut current = active_spec(&self.settings);
        if presets().iter().any(|preset| preset.name == current.name) {
            current.name = format!("{} (mine)", current.name);
        }
        let mut fields = vec![Field::new("Theme name:", current.name.clone())];
        let mut copy = current.clone();
        for (label, value) in copy.colours_mut() {
            fields.push(Field::new(&format!("{label}:"), value.clone()));
        }
        fields.push(Field::new("Chrome font:", current.ui_font.clone()));
        fields.push(Field::new("Chrome size (pt):", current.ui_size.to_string()));

        let frame = self.hwnd;
        let mut form = Form::new("Theme Editor").note(
            "Colours as #rrggbb. Click a colour box, then Pick Colour... to choose \
             it. Preview shows the theme at once; OK saves it as one of your \
             themes (View > Theme), Cancel goes back.",
        );
        for field in fields {
            form = form.field(field);
        }
        let values = form
            .extra(
                "Pick Colour...",
                Box::new(|owner, values| {
                    let Some(index) = crate::wnd::form::last_focused_field() else {
                        return;
                    };
                    if index == 0 || index > 12 {
                        return;
                    }
                    let initial = parse_hex_color(&values[index])
                        .map(|(r, g, b)| theme::rgb(r, g, b))
                        .unwrap_or(0);
                    if let Some(colour) = crate::wnd::format::choose_color(owner, initial) {
                        values[index] = format_hex_color((
                            (colour & 0xFF) as u8,
                            ((colour >> 8) & 0xFF) as u8,
                            ((colour >> 16) & 0xFF) as u8,
                        ));
                    }
                }),
            )
            .extra(
                "Preview",
                Box::new(move |_, values| {
                    let spec = spec_from(values);
                    // SAFETY: the frame outlives the modal form; the same
                    // re-entrancy every live preview in Nodi uses.
                    if let Some(app) = unsafe { crate::wnd::app_mut(frame) } {
                        app.preview_theme(spec);
                    }
                }),
            )
            .extra(
                "Copy as Text",
                Box::new(move |owner, values| {
                    let mut spec = spec_from(values);
                    spec.clamp();
                    clipboard::set_text(owner, &spec.to_json());
                }),
            )
            .extra(
                "Paste Text",
                Box::new(move |owner, values| {
                    let text = image::clipboard_text(owner).unwrap_or_default();
                    if let Some(spec) = ThemeSpec::from_json(&text) {
                        fill_values(values, &spec);
                    }
                }),
            )
            .show(self.hwnd);
        // Whatever was previewed goes away again.
        self.settings = previous;
        match values {
            Some(values) => {
                let name = self.save_my_theme(spec_from(&values));
                self.set_theme_name(&name);
            }
            None => {
                theme::apply(self);
                menu::sync_settings(self);
            }
        }
    }

    /// Shows `spec` without saving it (Theme Editor > Preview).
    fn preview_theme(&mut self, mut spec: ThemeSpec) {
        spec.clamp();
        spec.name = PREVIEW.to_owned();
        self.settings
            .my_themes
            .retain(|theme| theme.name != PREVIEW);
        self.settings.my_themes.push(spec);
        self.settings.theme_name = PREVIEW.to_owned();
        theme::apply(self);
    }

    /// View > Theme > Delete My Theme: removes the active saved theme.
    pub fn delete_my_theme(&mut self) {
        let name = self.settings.theme_name.clone();
        if !self
            .settings
            .my_themes
            .iter()
            .any(|theme| theme.name == name)
        {
            dialogs::info(
                self.hwnd,
                "The active theme is a built-in one. Choose one of your own themes first.",
            );
            return;
        }
        if !dialogs::ask_yes_no(self.hwnd, &format!("Delete your theme \"{name}\"?")) {
            return;
        }
        self.settings.my_themes.retain(|theme| theme.name != name);
        self.set_theme_name("Light");
    }
}

/// Theme from the editor's values (name, 12 colours, font, size).
fn spec_from(values: &[String]) -> ThemeSpec {
    let mut spec = ThemeSpec {
        name: values.first().cloned().unwrap_or_default(),
        ..ThemeSpec::default()
    };
    for (index, (_, slot)) in spec.colours_mut().into_iter().enumerate() {
        if let Some(value) = values.get(index + 1) {
            *slot = value.trim().to_owned();
        }
    }
    spec.ui_font = values
        .get(13)
        .cloned()
        .unwrap_or_default()
        .trim()
        .to_owned();
    spec.ui_size = values
        .get(14)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(9);
    spec
}

fn fill_values(values: &mut [String], spec: &ThemeSpec) {
    let mut copy = spec.clone();
    if let Some(name) = values.first_mut() {
        *name = copy.name.clone();
    }
    for (index, (_, value)) in copy.colours_mut().into_iter().enumerate() {
        if let Some(slot) = values.get_mut(index + 1) {
            *slot = value.clone();
        }
    }
    if let Some(slot) = values.get_mut(13) {
        *slot = spec.ui_font.clone();
    }
    if let Some(slot) = values.get_mut(14) {
        *slot = spec.ui_size.to_string();
    }
}
