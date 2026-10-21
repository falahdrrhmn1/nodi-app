//! Menu bar, command ids, the accelerator table and the shortcut list shown in
//! Help > Keyboard Shortcuts (`SHORTCUTS` is the single source of truth for
//! the documented keys).

use std::ptr;

use nodi_core::NodeKind;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_DOWN, VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_NEXT, VK_OEM_3, VK_OEM_4, VK_OEM_6,
    VK_OEM_MINUS, VK_OEM_PLUS, VK_PRIOR, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CheckMenuItem, CreateAcceleratorTableW, CreateMenu, CreatePopupMenu, PostMessageW,
    ACCEL, FALT, FCONTROL, FSHIFT, FVIRTKEY, HACCEL, HMENU, MF_BYCOMMAND, MF_CHECKED, MF_POPUP,
    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, WM_CLOSE,
};

use crate::app::{self, App};
use crate::win;
use crate::wnd::{dialogs, editor, find, format, image, link, status, table};

pub const IDM_FILE_NEW: u16 = 100;
pub const IDM_FILE_SAVE: u16 = 101;
pub const IDM_FILE_CLOSE_TAB: u16 = 102;
pub const IDM_FILE_EXIT: u16 = 103;
pub const IDM_FILE_EXPORT_MD: u16 = 104;

pub const IDM_EDIT_UNDO: u16 = 110;
pub const IDM_EDIT_REDO: u16 = 111;
pub const IDM_EDIT_CUT: u16 = 112;
pub const IDM_EDIT_COPY: u16 = 113;
pub const IDM_EDIT_PASTE: u16 = 114;
pub const IDM_EDIT_SELECT_ALL: u16 = 115;
pub const IDM_EDIT_PASTE_PLAIN: u16 = 116;
pub const IDM_EDIT_DUPLICATE_LINE: u16 = 117;
pub const IDM_EDIT_GOTO_LINE: u16 = 119;

pub const IDM_FORMAT_BOLD: u16 = 120;
pub const IDM_FORMAT_ITALIC: u16 = 121;
pub const IDM_FORMAT_UNDERLINE: u16 = 122;
pub const IDM_FORMAT_STRIKEOUT: u16 = 123;
pub const IDM_FORMAT_INLINE_CODE: u16 = 124;
pub const IDM_FORMAT_HIGHLIGHT: u16 = 125;
pub const IDM_FORMAT_COLOR: u16 = 126;
pub const IDM_FORMAT_FONT: u16 = 127;
pub const IDM_FORMAT_BIGGER: u16 = 128;
pub const IDM_FORMAT_SMALLER: u16 = 129;
pub const IDM_FORMAT_H1: u16 = 210;
pub const IDM_FORMAT_H2: u16 = 211;
pub const IDM_FORMAT_H3: u16 = 212;
pub const IDM_FORMAT_NORMAL: u16 = 213;
pub const IDM_FORMAT_BULLETS: u16 = 214;
pub const IDM_FORMAT_NUMBERS: u16 = 215;
pub const IDM_FORMAT_LEFT: u16 = 216;
pub const IDM_FORMAT_CENTER: u16 = 217;
pub const IDM_FORMAT_RIGHT: u16 = 218;
pub const IDM_FORMAT_JUSTIFY: u16 = 219;
pub const IDM_FORMAT_CLEAR: u16 = 220;
pub const IDM_FORMAT_INDENT: u16 = 221;
pub const IDM_FORMAT_OUTDENT: u16 = 222;

pub const IDM_TABLE_ROW_ABOVE: u16 = 230;
pub const IDM_TABLE_ROW_BELOW: u16 = 231;
pub const IDM_TABLE_COL_LEFT: u16 = 232;
pub const IDM_TABLE_COL_RIGHT: u16 = 233;
pub const IDM_TABLE_DEL_ROW: u16 = 234;
pub const IDM_TABLE_DEL_COL: u16 = 235;

pub const IDM_IMAGE_SIZE: u16 = 240;
pub const IDM_IMAGE_25: u16 = 241;
pub const IDM_IMAGE_50: u16 = 242;
pub const IDM_IMAGE_75: u16 = 243;
pub const IDM_IMAGE_100: u16 = 244;
pub const IDM_IMAGE_FIT: u16 = 245;
pub const IDM_IMAGE_SAVE: u16 = 246;

pub const IDM_VIEW_TOOLBAR: u16 = 130;
pub const IDM_VIEW_STATUS: u16 = 131;
pub const IDM_VIEW_EXPAND_ALL: u16 = 132;
pub const IDM_VIEW_COLLAPSE_ALL: u16 = 133;
pub const IDM_VIEW_WORD_WRAP: u16 = 134;
pub const IDM_VIEW_ZOOM_IN: u16 = 135;
pub const IDM_VIEW_ZOOM_OUT: u16 = 136;
pub const IDM_VIEW_ZOOM_RESET: u16 = 137;
pub const IDM_VIEW_MARKDOWN: u16 = 138;

pub const IDM_HELP_ABOUT: u16 = 140;
pub const IDM_HELP_SHORTCUTS: u16 = 141;

pub const IDM_TAB_NEXT: u16 = 150;
pub const IDM_TAB_PREV: u16 = 151;

pub const IDM_SEARCH_GOTO_NOTE: u16 = 160;
pub const IDM_SEARCH_FIND: u16 = 161;
pub const IDM_SEARCH_FIND_NEXT: u16 = 162;
pub const IDM_SEARCH_FIND_PREV: u16 = 163;
pub const IDM_SEARCH_REPLACE: u16 = 164;
pub const IDM_SEARCH_ALL_NOTES: u16 = 165;

pub const IDM_NOTES_NEW: u16 = 170;
pub const IDM_NOTES_OPEN: u16 = 171;
pub const IDM_NOTES_NEW_CHILD: u16 = 172;
pub const IDM_NOTES_NEW_SIBLING: u16 = 173;
pub const IDM_NOTES_NEW_FOLDER: u16 = 174;
pub const IDM_NOTES_RENAME: u16 = 175;
pub const IDM_NOTES_ICON: u16 = 176;
pub const IDM_NOTES_DUPLICATE: u16 = 177;
pub const IDM_NOTES_TRASH: u16 = 178;
pub const IDM_NOTES_TRASH_VIEW: u16 = 179;
pub const IDM_NOTES_PROPERTIES: u16 = 180;
pub const IDM_NOTES_MOVE_UP: u16 = 181;
pub const IDM_NOTES_MOVE_DOWN: u16 = 182;
pub const IDM_NOTES_COPY_LINK: u16 = 183;
pub const IDM_NOTES_MOVE_ROOT: u16 = 184;

pub const IDM_TOOLS_BACKUP: u16 = 190;
pub const IDM_TOOLS_OPEN_DATA: u16 = 191;

pub const IDM_INSERT_IMAGE: u16 = 200;
pub const IDM_INSERT_TABLE: u16 = 201;
pub const IDM_INSERT_DATETIME: u16 = 203;
pub const IDM_INSERT_LINK: u16 = 204;
pub const IDM_INSERT_CODE_BLOCK: u16 = 205;
pub const IDM_INSERT_FORMULA: u16 = 206;

/// Every documented shortcut: (keys, action). Shown by Help > Keyboard
/// Shortcuts and mirrored in docs/SHORTCUTS.md.
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+N", "New note (next to the selected one)"),
    ("Ctrl+Shift+N", "New child note"),
    ("Ctrl+S", "Save now (autosave runs anyway)"),
    ("Ctrl+W / Ctrl+F4", "Close tab"),
    ("Ctrl+Tab / Ctrl+PgDn", "Next tab"),
    ("Ctrl+Shift+Tab / Ctrl+PgUp", "Previous tab"),
    ("Ctrl+Z / Ctrl+Y", "Undo / Redo"),
    (
        "Ctrl+X / C / V",
        "Cut / Copy / Paste (also pastes screenshots)",
    ),
    ("Ctrl+Shift+V", "Paste as plain text"),
    ("Ctrl+A", "Select all"),
    ("Ctrl+D", "Duplicate line"),
    ("Ctrl+G", "Go to line"),
    ("Enter", "New line with the same indentation"),
    ("Shift+Enter", "Line break without indentation"),
    ("Tab / Shift+Tab", "Indent / outdent (whole selected lines)"),
    ("Ctrl+B / I / U", "Bold / Italic / Underline"),
    ("Alt+Shift+5", "Strikethrough"),
    ("Ctrl+`", "Inline code"),
    ("Ctrl+Shift+C", "Code block"),
    ("Ctrl+Shift+H", "Highlight"),
    ("Ctrl+] / Ctrl+[", "Bigger / smaller text"),
    ("Ctrl+Alt+1 / 2 / 3", "Heading 1 / 2 / 3"),
    ("Ctrl+Alt+0", "Normal text"),
    ("Ctrl+Shift+8 / 7", "Bulleted / numbered list"),
    (
        "Ctrl+L / E / R / J",
        "Align left / center / right / justify",
    ),
    ("Ctrl+Space", "Clear character formatting"),
    ("Ctrl+K", "Insert or edit link"),
    ("Ctrl+Click", "Follow a link"),
    ("Ctrl+Shift+I", "Insert image"),
    ("Ctrl+Shift+T", "Insert table"),
    ("Alt+=", "Insert formula"),
    ("F5", "Insert date and time"),
    ("Ctrl+F", "Find in note"),
    ("F3 / Shift+F3", "Find next / previous"),
    ("Ctrl+H", "Replace"),
    ("Ctrl+O", "Go to note by title"),
    ("Ctrl+Shift+F", "Search in all notes"),
    (
        "Ctrl+= / Ctrl+- / Ctrl+0",
        "Zoom in / out / reset (or Ctrl+mouse wheel)",
    ),
    ("Ctrl+Shift+M", "Show the note as Markdown (read-only)"),
    ("Double-click a formula", "Edit the formula"),
    ("F2", "Rename note"),
    ("Del (in the tree)", "Move note to Trash"),
    ("Alt+Up / Alt+Down", "Move note up / down"),
    ("F1", "This list"),
];

/// Builds the menu bar. Only commands that work are listed.
pub fn create_menu() -> HMENU {
    // SAFETY: every call creates/edits menus we own; strings are temporaries
    // that live for the duration of each call.
    unsafe {
        let bar = CreateMenu();
        if bar.is_null() {
            return ptr::null_mut();
        }

        let file = popup(&[
            Item::Cmd(IDM_FILE_NEW, "&New Note\tCtrl+N"),
            Item::Cmd(IDM_FILE_SAVE, "&Save\tCtrl+S"),
            Item::Cmd(IDM_FILE_CLOSE_TAB, "&Close Tab\tCtrl+W"),
            Item::Sep,
            Item::Cmd(IDM_FILE_EXPORT_MD, "Export Note as &Markdown..."),
            Item::Sep,
            Item::Cmd(IDM_FILE_EXIT, "E&xit"),
        ]);
        attach(bar, file, "&File");

        let edit = popup(&[
            Item::Cmd(IDM_EDIT_UNDO, "&Undo\tCtrl+Z"),
            Item::Cmd(IDM_EDIT_REDO, "&Redo\tCtrl+Y"),
            Item::Sep,
            Item::Cmd(IDM_EDIT_CUT, "Cu&t\tCtrl+X"),
            Item::Cmd(IDM_EDIT_COPY, "&Copy\tCtrl+C"),
            Item::Cmd(IDM_EDIT_PASTE, "&Paste\tCtrl+V"),
            Item::Cmd(IDM_EDIT_PASTE_PLAIN, "Paste as Plain &Text\tCtrl+Shift+V"),
            Item::Sep,
            Item::Cmd(IDM_EDIT_SELECT_ALL, "Select &All\tCtrl+A"),
            Item::Cmd(IDM_EDIT_DUPLICATE_LINE, "&Duplicate Line\tCtrl+D"),
            Item::Cmd(IDM_EDIT_GOTO_LINE, "&Go to Line...\tCtrl+G"),
        ]);
        attach(bar, edit, "&Edit");

        let search = popup(&[
            Item::Cmd(IDM_SEARCH_FIND, "&Find...\tCtrl+F"),
            Item::Cmd(IDM_SEARCH_FIND_NEXT, "Find &Next\tF3"),
            Item::Cmd(IDM_SEARCH_FIND_PREV, "Find &Previous\tShift+F3"),
            Item::Cmd(IDM_SEARCH_REPLACE, "&Replace...\tCtrl+H"),
            Item::Sep,
            Item::Cmd(IDM_SEARCH_GOTO_NOTE, "&Go to Note...\tCtrl+O"),
            Item::Cmd(
                IDM_SEARCH_ALL_NOTES,
                "Search in &All Notes...\tCtrl+Shift+F",
            ),
        ]);
        attach(bar, search, "&Search");

        let view = popup(&[
            Item::Checked(IDM_VIEW_TOOLBAR, "&Toolbar"),
            Item::Checked(IDM_VIEW_STATUS, "&Status Bar"),
            Item::Checked(IDM_VIEW_WORD_WRAP, "&Word Wrap"),
            Item::Sep,
            Item::Cmd(IDM_VIEW_ZOOM_IN, "Zoom &In\tCtrl+="),
            Item::Cmd(IDM_VIEW_ZOOM_OUT, "Zoom &Out\tCtrl+-"),
            Item::Cmd(IDM_VIEW_ZOOM_RESET, "&Reset Zoom\tCtrl+0"),
            Item::Sep,
            Item::Cmd(
                IDM_VIEW_MARKDOWN,
                "&Markdown Source (read-only)\tCtrl+Shift+M",
            ),
            Item::Sep,
            Item::Cmd(IDM_VIEW_EXPAND_ALL, "&Expand All Notes"),
            Item::Cmd(IDM_VIEW_COLLAPSE_ALL, "&Collapse All Notes"),
        ]);
        attach(bar, view, "&View");

        let insert = popup(&[
            Item::Cmd(IDM_INSERT_IMAGE, "&Image...\tCtrl+Shift+I"),
            Item::Cmd(IDM_INSERT_TABLE, "&Table...\tCtrl+Shift+T"),
            Item::Cmd(IDM_INSERT_LINK, "&Link...\tCtrl+K"),
            Item::Cmd(IDM_INSERT_CODE_BLOCK, "&Code Block\tCtrl+Shift+C"),
            Item::Cmd(IDM_INSERT_FORMULA, "&Formula...\tAlt+="),
            Item::Sep,
            Item::Cmd(IDM_INSERT_DATETIME, "&Date and Time\tF5"),
        ]);
        attach(bar, insert, "&Insert");

        let table_menu = popup(&[
            Item::Cmd(IDM_TABLE_ROW_ABOVE, "Insert Row &Above"),
            Item::Cmd(IDM_TABLE_ROW_BELOW, "Insert Row &Below"),
            Item::Cmd(IDM_TABLE_COL_LEFT, "Insert Column &Left"),
            Item::Cmd(IDM_TABLE_COL_RIGHT, "Insert Column &Right"),
            Item::Sep,
            Item::Cmd(IDM_TABLE_DEL_ROW, "&Delete Row"),
            Item::Cmd(IDM_TABLE_DEL_COL, "Delete &Column"),
        ]);
        let image_menu = popup(&[
            Item::Cmd(IDM_IMAGE_SIZE, "&Size..."),
            Item::Cmd(IDM_IMAGE_25, "&25%"),
            Item::Cmd(IDM_IMAGE_50, "&50%"),
            Item::Cmd(IDM_IMAGE_75, "&75%"),
            Item::Cmd(IDM_IMAGE_100, "&100% (original)"),
            Item::Cmd(IDM_IMAGE_FIT, "&Fit to Width"),
            Item::Sep,
            Item::Cmd(IDM_IMAGE_SAVE, "Save Image &As..."),
        ]);
        let format = popup(&[
            Item::Cmd(IDM_FORMAT_BOLD, "&Bold\tCtrl+B"),
            Item::Cmd(IDM_FORMAT_ITALIC, "&Italic\tCtrl+I"),
            Item::Cmd(IDM_FORMAT_UNDERLINE, "&Underline\tCtrl+U"),
            Item::Cmd(IDM_FORMAT_STRIKEOUT, "&Strikethrough\tAlt+Shift+5"),
            Item::Cmd(IDM_FORMAT_INLINE_CODE, "Inline &Code\tCtrl+`"),
            Item::Cmd(IDM_FORMAT_HIGHLIGHT, "&Highlight\tCtrl+Shift+H"),
            Item::Cmd(IDM_FORMAT_COLOR, "Text C&olor..."),
            Item::Cmd(IDM_FORMAT_FONT, "&Font..."),
            Item::Cmd(IDM_FORMAT_BIGGER, "Bigger Text\tCtrl+]"),
            Item::Cmd(IDM_FORMAT_SMALLER, "Smaller Text\tCtrl+["),
            Item::Cmd(IDM_FORMAT_CLEAR, "Clear Formatting\tCtrl+Space"),
            Item::Sep,
            Item::Cmd(IDM_FORMAT_H1, "Heading &1\tCtrl+Alt+1"),
            Item::Cmd(IDM_FORMAT_H2, "Heading &2\tCtrl+Alt+2"),
            Item::Cmd(IDM_FORMAT_H3, "Heading &3\tCtrl+Alt+3"),
            Item::Cmd(IDM_FORMAT_NORMAL, "&Normal Text\tCtrl+Alt+0"),
            Item::Sep,
            Item::Cmd(IDM_FORMAT_BULLETS, "Bulleted &List\tCtrl+Shift+8"),
            Item::Cmd(IDM_FORMAT_NUMBERS, "Numbered Lis&t\tCtrl+Shift+7"),
            Item::Cmd(IDM_FORMAT_INDENT, "Indent\tTab"),
            Item::Cmd(IDM_FORMAT_OUTDENT, "Outdent\tShift+Tab"),
            Item::Sep,
            Item::Cmd(IDM_FORMAT_LEFT, "Align Left\tCtrl+L"),
            Item::Cmd(IDM_FORMAT_CENTER, "Align Center\tCtrl+E"),
            Item::Cmd(IDM_FORMAT_RIGHT, "Align Right\tCtrl+R"),
            Item::Cmd(IDM_FORMAT_JUSTIFY, "Justify\tCtrl+J"),
            Item::Sep,
            Item::Popup(table_menu, "T&able"),
            Item::Popup(image_menu, "I&mage"),
        ]);
        attach(bar, format, "F&ormat");

        let notes = popup(&[
            Item::Cmd(IDM_NOTES_NEW, "&New Note\tCtrl+N"),
            Item::Cmd(IDM_NOTES_NEW_CHILD, "New &Child Note\tCtrl+Shift+N"),
            Item::Cmd(IDM_NOTES_NEW_SIBLING, "New &Sibling Note"),
            Item::Cmd(IDM_NOTES_NEW_FOLDER, "New F&older"),
            Item::Sep,
            Item::Cmd(IDM_NOTES_RENAME, "&Rename\tF2"),
            Item::Cmd(IDM_NOTES_ICON, "Change &Icon..."),
            Item::Cmd(IDM_NOTES_DUPLICATE, "&Duplicate"),
            Item::Cmd(IDM_NOTES_MOVE_UP, "Move &Up\tAlt+Up"),
            Item::Cmd(IDM_NOTES_MOVE_DOWN, "Move Do&wn\tAlt+Down"),
            Item::Cmd(IDM_NOTES_MOVE_ROOT, "Move to &Top Level"),
            Item::Cmd(IDM_NOTES_COPY_LINK, "Copy Internal &Link"),
            Item::Sep,
            Item::Cmd(IDM_NOTES_TRASH, "Move to &Trash\tDel"),
            Item::Cmd(IDM_NOTES_TRASH_VIEW, "Tr&ash..."),
            Item::Cmd(IDM_NOTES_PROPERTIES, "&Properties"),
        ]);
        attach(bar, notes, "&Notes");

        let tools = popup(&[
            Item::Cmd(IDM_TOOLS_BACKUP, "&Backup Now"),
            Item::Cmd(IDM_TOOLS_OPEN_DATA, "&Open Data Folder"),
        ]);
        attach(bar, tools, "&Tools");

        let help = popup(&[
            Item::Cmd(IDM_HELP_SHORTCUTS, "&Keyboard Shortcuts\tF1"),
            Item::Cmd(IDM_HELP_ABOUT, "&About Nodi"),
        ]);
        attach(bar, help, "&Help");

        bar
    }
}

pub enum Item {
    Cmd(u16, &'static str),
    Checked(u16, &'static str),
    Popup(HMENU, &'static str),
    Sep,
}

/// Creates a popup menu from `items`.
pub fn popup(items: &[Item]) -> HMENU {
    // SAFETY: we own the new menu; strings outlive each call.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return menu;
        }
        for item in items {
            match item {
                Item::Cmd(id, label) => {
                    AppendMenuW(menu, MF_STRING, *id as usize, win::wide(label).as_ptr());
                }
                Item::Checked(id, label) => {
                    AppendMenuW(
                        menu,
                        MF_STRING | MF_CHECKED,
                        *id as usize,
                        win::wide(label).as_ptr(),
                    );
                }
                Item::Popup(sub, label) => {
                    AppendMenuW(menu, MF_POPUP, *sub as usize, win::wide(label).as_ptr());
                }
                Item::Sep => {
                    AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                }
            }
        }
        menu
    }
}

unsafe fn attach(bar: HMENU, menu: HMENU, label: &str) {
    if !menu.is_null() {
        AppendMenuW(bar, MF_POPUP, menu as usize, win::wide(label).as_ptr());
    }
}

pub fn create_accel() -> HACCEL {
    let ctrl = FCONTROL;
    let ctrl_shift = FCONTROL | FSHIFT;
    let ctrl_alt = FCONTROL | FALT;
    let key = |ch: u8| u16::from(ch);
    let accels = [
        accel(ctrl, key(b'N'), IDM_FILE_NEW),
        accel(ctrl_shift, key(b'N'), IDM_NOTES_NEW_CHILD),
        accel(ctrl, key(b'S'), IDM_FILE_SAVE),
        accel(ctrl, key(b'W'), IDM_FILE_CLOSE_TAB),
        accel(ctrl, VK_F4, IDM_FILE_CLOSE_TAB),
        accel(ctrl, key(b'Z'), IDM_EDIT_UNDO),
        accel(ctrl, key(b'Y'), IDM_EDIT_REDO),
        accel(ctrl, key(b'D'), IDM_EDIT_DUPLICATE_LINE),
        accel(ctrl, key(b'G'), IDM_EDIT_GOTO_LINE),
        accel(ctrl, key(b'B'), IDM_FORMAT_BOLD),
        accel(ctrl, key(b'I'), IDM_FORMAT_ITALIC),
        accel(ctrl, key(b'U'), IDM_FORMAT_UNDERLINE),
        accel(FALT | FSHIFT, key(b'5'), IDM_FORMAT_STRIKEOUT),
        accel(ctrl, VK_OEM_3, IDM_FORMAT_INLINE_CODE),
        accel(ctrl_shift, key(b'C'), IDM_INSERT_CODE_BLOCK),
        accel(ctrl_shift, key(b'H'), IDM_FORMAT_HIGHLIGHT),
        accel(ctrl, VK_OEM_6, IDM_FORMAT_BIGGER),
        accel(ctrl, VK_OEM_4, IDM_FORMAT_SMALLER),
        accel(ctrl, VK_SPACE, IDM_FORMAT_CLEAR),
        accel(ctrl_alt, key(b'1'), IDM_FORMAT_H1),
        accel(ctrl_alt, key(b'2'), IDM_FORMAT_H2),
        accel(ctrl_alt, key(b'3'), IDM_FORMAT_H3),
        accel(ctrl_alt, key(b'0'), IDM_FORMAT_NORMAL),
        accel(ctrl_shift, key(b'8'), IDM_FORMAT_BULLETS),
        accel(ctrl_shift, key(b'7'), IDM_FORMAT_NUMBERS),
        accel(ctrl, key(b'L'), IDM_FORMAT_LEFT),
        accel(ctrl, key(b'E'), IDM_FORMAT_CENTER),
        accel(ctrl, key(b'R'), IDM_FORMAT_RIGHT),
        accel(ctrl, key(b'J'), IDM_FORMAT_JUSTIFY),
        accel(ctrl, key(b'K'), IDM_INSERT_LINK),
        accel(ctrl_shift, key(b'I'), IDM_INSERT_IMAGE),
        accel(ctrl_shift, key(b'T'), IDM_INSERT_TABLE),
        accel(FALT, VK_OEM_PLUS, IDM_INSERT_FORMULA),
        accel(0, VK_F5, IDM_INSERT_DATETIME),
        accel(ctrl, key(b'F'), IDM_SEARCH_FIND),
        accel(0, VK_F3, IDM_SEARCH_FIND_NEXT),
        accel(FSHIFT, VK_F3, IDM_SEARCH_FIND_PREV),
        accel(ctrl, key(b'H'), IDM_SEARCH_REPLACE),
        accel(ctrl, key(b'O'), IDM_SEARCH_GOTO_NOTE),
        accel(ctrl_shift, key(b'F'), IDM_SEARCH_ALL_NOTES),
        accel(ctrl, VK_OEM_PLUS, IDM_VIEW_ZOOM_IN),
        accel(ctrl, VK_OEM_MINUS, IDM_VIEW_ZOOM_OUT),
        accel(ctrl, key(b'0'), IDM_VIEW_ZOOM_RESET),
        accel(ctrl_shift, key(b'M'), IDM_VIEW_MARKDOWN),
        accel(0, VK_F2, IDM_NOTES_RENAME),
        accel(FALT, VK_UP, IDM_NOTES_MOVE_UP),
        accel(FALT, VK_DOWN, IDM_NOTES_MOVE_DOWN),
        accel(ctrl, VK_TAB, IDM_TAB_NEXT),
        accel(ctrl_shift, VK_TAB, IDM_TAB_PREV),
        accel(ctrl, VK_NEXT, IDM_TAB_NEXT),
        accel(ctrl, VK_PRIOR, IDM_TAB_PREV),
        accel(0, VK_F1, IDM_HELP_SHORTCUTS),
    ];
    // SAFETY: `accels` is a valid array for the duration of the call.
    unsafe { CreateAcceleratorTableW(accels.as_ptr(), accels.len() as i32) }
}

fn accel(modifiers: u8, key: u16, command: u16) -> ACCEL {
    ACCEL {
        fVirt: modifiers | FVIRTKEY,
        key,
        cmd: command,
    }
}

/// Shows a message (and keeps the editor focused) when an editor command
/// fails, e.g. "Select a picture first".
fn report(app: &App, outcome: Result<(), String>) {
    if let Err(message) = outcome {
        status::set_message(app, &message);
        dialogs::info(app.hwnd, &message);
    }
}

pub fn on_command(app: &mut App, id: u16) {
    match id {
        IDM_FILE_NEW => app.create_note(NodeKind::Note, false),
        IDM_FILE_SAVE => {
            app.save_active();
            status::set_message(app, "Saving...");
        }
        IDM_FILE_CLOSE_TAB => app.close_active_tab(),
        IDM_FILE_EXPORT_MD => app::export_markdown(app),
        IDM_FILE_EXIT => {
            // SAFETY: posting to our own top-level window.
            unsafe {
                PostMessageW(app.hwnd, WM_CLOSE, 0, 0);
            }
        }
        IDM_EDIT_UNDO | IDM_EDIT_REDO | IDM_EDIT_CUT | IDM_EDIT_COPY | IDM_EDIT_PASTE
        | IDM_EDIT_SELECT_ALL | IDM_EDIT_PASTE_PLAIN => editor::command(app, id),
        IDM_EDIT_DUPLICATE_LINE => with_editor(app, editor::duplicate_line),
        IDM_EDIT_GOTO_LINE => app::go_to_line(app),

        IDM_FORMAT_BOLD | IDM_FORMAT_ITALIC | IDM_FORMAT_UNDERLINE | IDM_FORMAT_STRIKEOUT => {
            editor::toggle_effect(app, id);
        }
        IDM_FORMAT_INLINE_CODE => with_editor(app, format::toggle_inline_code),
        IDM_FORMAT_HIGHLIGHT => with_editor(app, format::toggle_highlight),
        IDM_FORMAT_COLOR => {
            let owner = app.hwnd;
            with_editor(app, |editor| format::choose_text_color(editor, owner));
        }
        IDM_FORMAT_FONT => {
            let owner = app.hwnd;
            with_editor(app, |editor| format::choose_font(editor, owner));
        }
        IDM_FORMAT_BIGGER => with_editor(app, |editor| format::step_font_size(editor, true)),
        IDM_FORMAT_SMALLER => with_editor(app, |editor| format::step_font_size(editor, false)),
        IDM_FORMAT_CLEAR => with_editor(app, format::clear_formatting),
        IDM_FORMAT_H1 => with_editor(app, |editor| format::heading(editor, 1)),
        IDM_FORMAT_H2 => with_editor(app, |editor| format::heading(editor, 2)),
        IDM_FORMAT_H3 => with_editor(app, |editor| format::heading(editor, 3)),
        IDM_FORMAT_NORMAL => with_editor(app, |editor| format::heading(editor, 0)),
        IDM_FORMAT_BULLETS => with_editor(app, |editor| format::toggle_list(editor, false)),
        IDM_FORMAT_NUMBERS => with_editor(app, |editor| format::toggle_list(editor, true)),
        IDM_FORMAT_INDENT => with_editor(app, |editor| {
            if !crate::wnd::editkeys::tab_key(editor, false) {
                editor::replace_selection(editor, "\t", true);
            }
        }),
        IDM_FORMAT_OUTDENT => with_editor(app, |editor| {
            crate::wnd::editkeys::tab_key(editor, true);
        }),
        IDM_FORMAT_LEFT => with_editor(app, |editor| format::align(editor, format::PFA_LEFT)),
        IDM_FORMAT_CENTER => with_editor(app, |editor| format::align(editor, format::PFA_CENTER)),
        IDM_FORMAT_RIGHT => with_editor(app, |editor| format::align(editor, format::PFA_RIGHT)),
        IDM_FORMAT_JUSTIFY => {
            with_editor(app, |editor| format::align(editor, format::PFA_JUSTIFY));
        }

        IDM_TABLE_ROW_ABOVE => editor_result(app, |editor| table::insert_row(editor, false)),
        IDM_TABLE_ROW_BELOW => editor_result(app, |editor| table::insert_row(editor, true)),
        IDM_TABLE_COL_LEFT => editor_result(app, |editor| table::insert_column(editor, false)),
        IDM_TABLE_COL_RIGHT => editor_result(app, |editor| table::insert_column(editor, true)),
        IDM_TABLE_DEL_ROW => editor_result(app, table::delete_row),
        IDM_TABLE_DEL_COL => editor_result(app, table::delete_column),

        IDM_IMAGE_SIZE => {
            let owner = app.hwnd;
            editor_result(app, |editor| image::resize_dialog(editor, owner));
        }
        IDM_IMAGE_25 => editor_result(app, |editor| image::resize_percent(editor, 25)),
        IDM_IMAGE_50 => editor_result(app, |editor| image::resize_percent(editor, 50)),
        IDM_IMAGE_75 => editor_result(app, |editor| image::resize_percent(editor, 75)),
        IDM_IMAGE_100 => editor_result(app, |editor| image::resize_percent(editor, 100)),
        IDM_IMAGE_FIT => editor_result(app, image::fit_width),
        IDM_IMAGE_SAVE => {
            let owner = app.hwnd;
            if let Some(editor) = app.active_editor() {
                report(app, image::save_as(editor, owner));
            }
        }

        IDM_INSERT_IMAGE => app::insert_image_file(app),
        IDM_INSERT_TABLE => app::insert_table(app),
        IDM_INSERT_LINK => link::insert_or_edit(app),
        IDM_INSERT_CODE_BLOCK => with_editor(app, format::insert_code_block),
        IDM_INSERT_FORMULA => app::insert_formula(app),
        IDM_INSERT_DATETIME => with_editor(app, |editor| {
            editor::replace_selection(editor, &app::local_date_time(), true);
        }),

        IDM_SEARCH_GOTO_NOTE => app::show_find_note(app),
        IDM_SEARCH_FIND => find::open(app, false),
        IDM_SEARCH_REPLACE => find::open(app, true),
        IDM_SEARCH_FIND_NEXT => find::find_again(app, true),
        IDM_SEARCH_FIND_PREV => find::find_again(app, false),
        IDM_SEARCH_ALL_NOTES => find::search_all(app),

        IDM_VIEW_TOOLBAR => {
            app.toolbar_visible = !app.toolbar_visible;
            check_item(app, id, app.toolbar_visible);
            crate::wnd::layout(app);
        }
        IDM_VIEW_STATUS => {
            app.status_visible = !app.status_visible;
            check_item(app, id, app.status_visible);
            crate::wnd::layout(app);
        }
        IDM_VIEW_WORD_WRAP => {
            app.word_wrap = !app.word_wrap;
            check_item(app, id, app.word_wrap);
            for tab in &app.tabs {
                editor::set_word_wrap(tab.editor, app.word_wrap);
            }
        }
        IDM_VIEW_ZOOM_IN => app::zoom(app, Some(true)),
        IDM_VIEW_ZOOM_OUT => app::zoom(app, Some(false)),
        IDM_VIEW_ZOOM_RESET => app::zoom(app, None),
        IDM_VIEW_MARKDOWN => app::show_markdown(app),
        IDM_VIEW_EXPAND_ALL => app.expand_all(),
        IDM_VIEW_COLLAPSE_ALL => app.collapse_all(),

        IDM_NOTES_NEW => {
            let as_child = app.context_note.is_some()
                && app
                    .context_note
                    .as_ref()
                    .and_then(|id| app.tree.get(id))
                    .is_some_and(|node| node.kind == NodeKind::Folder);
            app.create_note(NodeKind::Note, as_child);
        }
        IDM_NOTES_NEW_CHILD => app.create_note(NodeKind::Note, true),
        IDM_NOTES_NEW_SIBLING => app.create_note(NodeKind::Note, false),
        IDM_NOTES_NEW_FOLDER => {
            let as_child = app.context_note.is_some();
            app.create_note(NodeKind::Folder, as_child);
        }
        IDM_NOTES_OPEN => app::open_selected(app),
        IDM_NOTES_RENAME => app::rename_selected(app),
        IDM_NOTES_ICON => app::change_icon(app),
        IDM_NOTES_DUPLICATE => {
            if let Some(note) = app.context_note.clone().or_else(|| app.selected.clone()) {
                app.duplicate_node(note);
            }
        }
        IDM_NOTES_MOVE_UP => app.move_selected(-1),
        IDM_NOTES_MOVE_DOWN => app.move_selected(1),
        IDM_NOTES_MOVE_ROOT => app.move_selected_to_root(),
        IDM_NOTES_COPY_LINK => app::copy_internal_link(app),
        IDM_NOTES_TRASH => {
            if let Some(note) = app.context_note.clone().or_else(|| app.selected.clone()) {
                app.trash_node(note);
            }
        }
        IDM_NOTES_TRASH_VIEW => app::show_trash(app),
        IDM_NOTES_PROPERTIES => app::show_properties(app),
        IDM_TOOLS_BACKUP => app.backup_now(),
        IDM_TOOLS_OPEN_DATA => app.open_data_folder(),
        IDM_HELP_ABOUT => dialogs::about(app.hwnd),
        IDM_HELP_SHORTCUTS => dialogs::shortcuts(app.hwnd),
        IDM_TAB_NEXT => app.cycle_tab(1),
        IDM_TAB_PREV => app.cycle_tab(-1),
        _ => {}
    }
}

/// Runs an editor command on the active tab and marks it dirty (formatting
/// changes do not always raise `EN_CHANGE`).
fn with_editor(app: &mut App, command: impl FnOnce(windows_sys::Win32::Foundation::HWND)) {
    let Some(editor) = app.active_editor() else {
        status::set_message(app, "No note open");
        return;
    };
    command(editor);
    app.mark_dirty(editor);
    // SAFETY: focusing our own editor.
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(editor);
    }
}

fn editor_result(
    app: &mut App,
    command: impl FnOnce(windows_sys::Win32::Foundation::HWND) -> Result<(), String>,
) {
    let Some(editor) = app.active_editor() else {
        status::set_message(app, "No note open");
        return;
    };
    match command(editor) {
        Ok(()) => app.mark_dirty(editor),
        Err(message) => report(app, Err(message)),
    }
    // SAFETY: focusing our own editor.
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(editor);
    }
}

pub fn set_checked(app: &App, id: u16, checked: bool) {
    check_item(app, id, checked);
}

fn check_item(app: &App, id: u16, checked: bool) {
    if app.menu.is_null() {
        return;
    }
    // SAFETY: `app.menu` is the menu bar attached to our frame window.
    unsafe {
        CheckMenuItem(
            app.menu,
            id as u32,
            MF_BYCOMMAND | if checked { MF_CHECKED } else { MF_UNCHECKED },
        );
    }
}
