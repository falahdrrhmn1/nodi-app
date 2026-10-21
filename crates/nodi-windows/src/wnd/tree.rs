//! Notes Explorer sidebar (TreeView) and its caption for Phase 1.

use std::ptr;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::SystemServices::{SS_CENTERIMAGE, SS_LEFT, SS_NOPREFIX};
use windows_sys::Win32::UI::Controls::{
    HTREEITEM, TVIF_TEXT, TVINSERTSTRUCTW, TVINSERTSTRUCTW_0, TVITEMW, TVI_LAST, TVI_ROOT,
    TVM_EXPAND, TVM_INSERTITEMW, TVS_EDITLABELS, TVS_HASBUTTONS, TVS_HASLINES, TVS_LINESATROOT,
    TVS_SHOWSELALWAYS, WC_TREEVIEWW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, SendMessageW, SetWindowTextW, WS_CHILD, WS_EX_CLIENTEDGE, WS_VISIBLE,
};

use crate::app::App;
use crate::win;

const TVE_EXPAND_VALUE: u32 = 0x0002;

pub fn create(app: &App) -> HWND {
    let style = WS_CHILD
        | WS_VISIBLE
        | TVS_HASLINES
        | TVS_LINESATROOT
        | TVS_HASBUTTONS
        | TVS_SHOWSELALWAYS
        | TVS_EDITLABELS;

    // SAFETY: class name constant, `app.hwnd` is the live frame window.
    unsafe {
        CreateWindowExW(
            WS_EX_CLIENTEDGE,
            WC_TREEVIEWW,
            ptr::null(),
            style,
            0,
            0,
            0,
            0,
            app.hwnd,
            ptr::null_mut(),
            app.hinstance,
            ptr::null(),
        )
    }
}

/// Caption above the tree ("Notes Explorer").
pub fn create_header(app: &App) -> HWND {
    let class = win::wide("Static");
    let style = WS_CHILD | WS_VISIBLE | SS_LEFT | SS_CENTERIMAGE | SS_NOPREFIX;
    // SAFETY: constant class name; string buffer below outlives the calls.
    unsafe {
        let header = CreateWindowExW(
            0,
            class.as_ptr(),
            ptr::null(),
            style,
            0,
            0,
            0,
            0,
            app.hwnd,
            ptr::null_mut(),
            app.hinstance,
            ptr::null(),
        );
        if !header.is_null() {
            let text = win::wide("Notes Explorer");
            SetWindowTextW(header, text.as_ptr());
        }
        header
    }
}

/// Static demo content so the Phase 1 layout is visible.
pub fn populate(tree: HWND) {
    let all_notes = insert(tree, TVI_ROOT, "All Notes");
    insert(tree, all_notes, "Welcome to Nodi");
    insert(tree, all_notes, "Project Ideas");
    insert(tree, all_notes, "Meeting Notes");
    insert(tree, TVI_ROOT, "Trash");

    // SAFETY: `tree` is the live TreeView; `all_notes` is its root item.
    unsafe {
        SendMessageW(tree, TVM_EXPAND, TVE_EXPAND_VALUE as usize, all_notes);
    }
}

fn insert(tree: HWND, parent: HTREEITEM, text: &str) -> HTREEITEM {
    let mut buffer = win::wide(text);
    let item = TVITEMW {
        mask: TVIF_TEXT,
        hItem: 0,
        state: 0,
        stateMask: 0,
        pszText: buffer.as_mut_ptr(),
        cchTextMax: buffer.len() as i32,
        iImage: 0,
        iSelectedImage: 0,
        cChildren: 0,
        lParam: 0,
    };
    let insert = TVINSERTSTRUCTW {
        hParent: parent,
        hInsertAfter: TVI_LAST,
        Anonymous: TVINSERTSTRUCTW_0 { item },
    };

    // SAFETY: `item`/`insert` and `buffer` live for the duration of the call.
    unsafe {
        SendMessageW(
            tree,
            TVM_INSERTITEMW,
            0,
            &insert as *const TVINSERTSTRUCTW as isize,
        ) as HTREEITEM
    }
}
