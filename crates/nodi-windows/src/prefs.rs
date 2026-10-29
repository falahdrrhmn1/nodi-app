//! Tools > Preferences: default font and size, sounds, typing helpers and
//! the switches that also live in the View/Tools menus.

use crate::app::App;
use crate::wnd::form::{Field, Form};
use crate::wnd::{dialogs, editor, menu, status};

impl App {
    /// Hands the saved default font to the editors and the paste converters.
    pub fn apply_body_font(&self) {
        nodi_core::markdown_in::set_body(
            &self.settings.body_font,
            (self.settings.body_size as i32) * 2,
        );
    }

    /// Tools > Preferences...
    pub fn show_preferences(&mut self) {
        let settings = self.settings.clone();
        let Some(values) = Form::new("Preferences")
            .note(
                "The default font is used for new notes and new text. Notes you \
                 already wrote keep their fonts. Themes, the background pattern \
                 and the sort order are in the View menu.",
            )
            .field(Field::new("Default font:", settings.body_font.clone()))
            .field(Field::new(
                "Default size (pt):",
                settings.body_size.to_string(),
            ))
            .checkbox("Sounds (beeps and message-box sounds)", settings.sounds)
            .checkbox("Two quick spaces type \". \"", settings.double_space_period)
            .checkbox("Line numbers next to the text", settings.line_numbers)
            .checkbox("Ctrl+Up/Down scrolls the view", settings.ctrl_arrow_scroll)
            .checkbox("Spell check", settings.spell_check)
            .checkbox(
                "Keep the Markdown copy for AI agents",
                settings.markdown_mirror,
            )
            .show(self.hwnd)
        else {
            return;
        };
        let flag = |index: usize| values.get(index).is_some_and(|value| value == "1");
        let font = values[0].trim().to_owned();
        // "Fixedsys" (a bitmap font Rich Edit cannot draw) can still mean a
        // TrueType remake such as "Fixedsys Excelsior": take the first
        // installed font whose name starts with what was typed.
        let closest = (!font.is_empty() && !crate::wnd::toolbar::font_installed(self, &font))
            .then(|| crate::wnd::toolbar::closest_font(self, &font))
            .flatten();
        if let Some(found) = closest {
            dialogs::info(
                self.hwnd,
                &format!(
                    "\"{font}\" itself cannot be drawn by the note editor (an old bitmap font or not installed); using \"{found}\" instead."
                ),
            );
            self.settings.body_font = found;
        } else if !font.is_empty() && !crate::wnd::toolbar::font_installed(self, &font) {
            dialogs::info(
                self.hwnd,
                &format!(
                    "\"{font}\" cannot be used: it is not installed, or it is an old bitmap font (Fixedsys, Terminal, System…) that the note editor cannot draw. Choose a TrueType/OpenType font such as Consolas or Lucida Console; the default font stays."
                ),
            );
        } else if !font.is_empty() {
            self.settings.body_font = font;
        }
        match values[1].trim().parse::<u32>() {
            Ok(size) if (6..=72).contains(&size) => self.settings.body_size = size,
            _ => status::set_message(self, "Default size must be 6–72 pt; it was not changed"),
        }
        self.settings.sounds = flag(2);
        self.settings.double_space_period = flag(3);
        let line_numbers = flag(4);
        self.settings.ctrl_arrow_scroll = flag(5);
        let spell = flag(6);
        let mirror = flag(7);

        self.apply_body_font();
        dialogs::set_sounds(self.settings.sounds);
        for tab in &self.tabs {
            editor::set_beeps(tab.editor, self.settings.sounds);
        }
        if line_numbers != self.settings.line_numbers {
            self.settings.line_numbers = line_numbers;
            crate::wnd::layout(self);
        }
        self.save_settings();
        if spell != self.settings.spell_check {
            self.toggle_spell_check();
        }
        if mirror != self.settings.markdown_mirror {
            self.toggle_mirror();
        }
        menu::sync_settings(self);
        status::set_message(self, "Preferences saved");
    }
}
