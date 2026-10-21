//! Right-click menu inside a note. It always offers the clipboard commands
//! and adds picture, table or link commands when the click lands on one.

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, POINT, POINTL};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyMenu, SendMessageW, TrackPopupMenu, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
};

use crate::app::App;
use crate::wnd::editor::{self, CharRange};
use crate::wnd::menu::{self, popup, Item};
use crate::wnd::{image, link, table};

const EM_CHARFROMPOS: u32 = 0x00D7;
const EM_POSFROMCHAR: u32 = 0x00D6;
const IDM_OPEN_LINK: u16 = 900;

pub fn show_editor_menu(app: &mut App, editor: HWND, screen: Option<POINT>) {
    // Right-clicking outside the selection moves the caret there first, like
    // Word, so the menu applies to what was clicked.
    let point = match screen {
        Some(point) => point,
        None => caret_screen_point(editor),
    };
    if screen.is_some() {
        let mut client = point;
        // SAFETY: converting into our editor's client coordinates.
        unsafe {
            ScreenToClient(editor, &mut client);
        }
        let position = char_from_point(editor, client);
        let (from, to) = editor::selection(editor);
        if position < from || position > to {
            editor::set_selection(editor, position, position);
        }
    }

    let formula = image_is_formula(editor);
    let picture = !formula && image::pick_picture(editor).is_some();
    let in_table = table::caret_in_table(editor) && !crate::wnd::format::in_code(editor);
    let link_target = link_under_caret(editor);

    let mut items = vec![
        Item::Cmd(menu::IDM_EDIT_CUT, "Cu&t\tCtrl+X"),
        Item::Cmd(menu::IDM_EDIT_COPY, "&Copy\tCtrl+C"),
        Item::Cmd(menu::IDM_EDIT_PASTE, "&Paste\tCtrl+V"),
        Item::Cmd(
            menu::IDM_EDIT_PASTE_PLAIN,
            "Paste as Plain Te&xt\tCtrl+Shift+V",
        ),
        Item::Sep,
        Item::Cmd(menu::IDM_EDIT_SELECT_ALL, "Select &All\tCtrl+A"),
    ];
    if picture {
        items.extend([
            Item::Sep,
            Item::Cmd(menu::IDM_IMAGE_SIZE, "Image &Size..."),
            Item::Cmd(menu::IDM_IMAGE_25, "Image 25%"),
            Item::Cmd(menu::IDM_IMAGE_50, "Image 50%"),
            Item::Cmd(menu::IDM_IMAGE_75, "Image 75%"),
            Item::Cmd(menu::IDM_IMAGE_100, "Image 100% (original)"),
            Item::Cmd(menu::IDM_IMAGE_FIT, "Image: Fit to Width"),
            Item::Cmd(menu::IDM_FORMAT_LEFT, "Align Left"),
            Item::Cmd(menu::IDM_FORMAT_CENTER, "Align Center"),
            Item::Cmd(menu::IDM_FORMAT_RIGHT, "Align Right"),
            Item::Cmd(menu::IDM_IMAGE_SAVE, "Save Image As..."),
        ]);
    }
    if formula {
        items.extend([
            Item::Sep,
            Item::Cmd(menu::IDM_INSERT_FORMULA, "Edit &Formula...\tDouble-click"),
        ]);
    }
    if in_table {
        items.extend([
            Item::Sep,
            Item::Cmd(menu::IDM_TABLE_ROW_ABOVE, "Insert Row Above"),
            Item::Cmd(menu::IDM_TABLE_ROW_BELOW, "Insert Row Below"),
            Item::Cmd(menu::IDM_TABLE_COL_LEFT, "Insert Column Left"),
            Item::Cmd(menu::IDM_TABLE_COL_RIGHT, "Insert Column Right"),
            Item::Cmd(menu::IDM_TABLE_DEL_ROW, "Delete Row"),
            Item::Cmd(menu::IDM_TABLE_DEL_COL, "Delete Column"),
        ]);
    }
    items.push(Item::Sep);
    let link_target = link_target.filter(|_| !formula);
    if link_target.is_some() {
        items.push(Item::Cmd(IDM_OPEN_LINK, "&Open Link\tCtrl+Click"));
        items.push(Item::Cmd(menu::IDM_INSERT_LINK, "&Edit Link...\tCtrl+K"));
    } else {
        items.push(Item::Cmd(menu::IDM_INSERT_LINK, "Insert &Link...\tCtrl+K"));
    }

    let handle = popup(&items);
    if handle.is_null() {
        return;
    }
    // SAFETY: `handle` is our popup; the frame window receives nothing because
    // TPM_RETURNCMD hands the command back.
    let command = unsafe {
        let command = TrackPopupMenu(
            handle,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN,
            point.x,
            point.y,
            0,
            app.hwnd,
            ptr::null(),
        );
        DestroyMenu(handle);
        command as u16
    };
    match command {
        0 => {}
        IDM_OPEN_LINK => {
            if let Some(target) = link_target {
                link::follow(app, &target);
            }
        }
        menu::IDM_INSERT_LINK if link_target.is_some() => {
            select_link(editor);
            link::insert_or_edit(app);
        }
        other => menu::on_command(app, other),
    }
}

fn char_from_point(editor: HWND, client: POINT) -> i32 {
    let point = POINTL {
        x: client.x,
        y: client.y,
    };
    // SAFETY: Rich Edit reads the POINTL for the duration of the call.
    unsafe { SendMessageW(editor, EM_CHARFROMPOS, 0, &point as *const POINTL as isize) as i32 }
}

fn caret_screen_point(editor: HWND) -> POINT {
    let (caret, _) = editor::selection(editor);
    let mut point = POINTL { x: 0, y: 0 };
    // SAFETY: `point` is a valid out parameter.
    unsafe {
        SendMessageW(
            editor,
            EM_POSFROMCHAR,
            &mut point as *mut POINTL as usize,
            caret as isize,
        );
    }
    let mut screen = POINT {
        x: point.x,
        y: point.y + 16,
    };
    // SAFETY: converting from our editor's client coordinates.
    unsafe {
        ClientToScreen(editor, &mut screen);
    }
    screen
}

/// Target of the link at the caret, if the caret is inside one.
fn link_under_caret(editor: HWND) -> Option<String> {
    let (start, end) = link_range(editor)?;
    Some(link::target_of(
        editor,
        CharRange {
            cp_min: start,
            cp_max: end,
        },
    ))
}

/// Extent of the link run around the caret (CFE_LINK characters).
fn link_range(editor: HWND) -> Option<(i32, i32)> {
    const CFE_LINK: u32 = 0x0020;
    let (from, to) = editor::selection(editor);
    let is_link = |position: i32| -> bool {
        editor::set_selection(editor, position, position + 1);
        let format = editor::selection_format(editor);
        format.dw_effects & CFE_LINK != 0
    };
    let length = editor::text_length(editor);
    let probe = if from < length && is_link(from) {
        Some(from)
    } else if from > 0 && is_link(from - 1) {
        Some(from - 1)
    } else {
        None
    };
    let result = probe.map(|at| {
        let mut start = at;
        while start > 0 && is_link(start - 1) {
            start -= 1;
        }
        let mut end = at + 1;
        while end < length && is_link(end) {
            end += 1;
        }
        (start, end)
    });
    editor::set_selection(editor, from, to);
    result
}

fn select_link(editor: HWND) {
    if let Some((start, end)) = link_range(editor) {
        editor::set_selection(editor, start, end);
    }
}

/// True when the caret/selection is on a Nodi formula.
fn image_is_formula(editor: HWND) -> bool {
    crate::wnd::formula::selected_latex(editor).is_some()
}
