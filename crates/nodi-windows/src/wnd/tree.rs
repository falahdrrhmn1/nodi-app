//! Notes Explorer: the `TreeView` mirror of `Tree`, its shell-stock icon list,
//! the rename/expand/drag notifications and the node context menu.

use std::cell::Cell;
use std::ptr;

use nodi_core::tree::Position;
use nodi_core::{NodeKind, NoteId};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, CreateBitmap, CreateDIBSection, DeleteObject, InvalidateRect, ScreenToClient,
    UpdateWindow, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, RGBQUAD,
};
use windows_sys::Win32::System::SystemServices::{SS_CENTERIMAGE, SS_LEFT, SS_NOPREFIX};
use windows_sys::Win32::UI::Controls::{
    ImageList_BeginDrag, ImageList_Create, ImageList_DragEnter, ImageList_DragLeave,
    ImageList_DragMove, ImageList_DragShowNolock, ImageList_EndDrag, ImageList_ReplaceIcon,
    CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDRF_DODEFAULT, CDRF_NEWFONT, CDRF_NOTIFYITEMDRAW,
    ILC_COLOR32, ILC_MASK, NMHDR, NMTREEVIEWW, NMTVCUSTOMDRAW, NMTVDISPINFOW, NMTVKEYDOWN,
    NM_CUSTOMDRAW, NM_DBLCLK, NM_RCLICK, TVE_EXPAND, TVGN_CARET, TVGN_DROPHILITE,
    TVGN_FIRSTVISIBLE, TVGN_NEXTVISIBLE, TVGN_PREVIOUS, TVGN_ROOT, TVHITTESTINFO, TVHT_ONITEM,
    TVHT_ONITEMBUTTON, TVHT_ONITEMINDENT, TVHT_ONITEMRIGHT, TVIF_CHILDREN, TVIF_IMAGE,
    TVIF_SELECTEDIMAGE, TVIF_TEXT, TVINSERTSTRUCTW, TVINSERTSTRUCTW_0, TVIS_EXPANDED, TVITEMW,
    TVI_FIRST, TVI_LAST, TVI_ROOT, TVM_DELETEITEM, TVM_EDITLABELW, TVM_ENSUREVISIBLE, TVM_EXPAND,
    TVM_GETEDITCONTROL, TVM_GETITEMRECT, TVM_GETITEMW, TVM_GETNEXTITEM, TVM_HITTEST,
    TVM_INSERTITEMW, TVM_SELECTITEM, TVM_SETIMAGELIST, TVM_SETINSERTMARK, TVM_SETITEMW,
    TVN_BEGINDRAGW, TVN_BEGINLABELEDITW, TVN_ENDLABELEDITW, TVN_ITEMEXPANDEDW, TVN_KEYDOWN,
    TVN_SELCHANGEDW, TVSIL_NORMAL, TVS_EDITLABELS, TVS_HASBUTTONS, TVS_HASLINES, TVS_LINESATROOT,
    TVS_SHOWSELALWAYS, WC_TREEVIEWW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, SetFocus, VK_APPS, VK_DELETE, VK_ESCAPE, VK_RETURN,
};
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, ExtractIconExW, SHGetStockIconInfo, SetWindowSubclass, SHGSI_ICON,
    SHGSI_SMALLICON, SHSTOCKICONID, SHSTOCKICONINFO, SIID_APPLICATION, SIID_DOCASSOC,
    SIID_DOCNOASSOC, SIID_FOLDER, SIID_HELP, SIID_INFO, SIID_RECYCLER, SIID_SHIELD, SIID_SOFTWARE,
    SIID_WARNING,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DestroyIcon, DestroyMenu,
    GetClientRect, GetCursorPos, KillTimer, LoadImageW, PostMessageW, SendMessageW,
    SetMenuDefaultItem, SetTimer, SetWindowTextW, TrackPopupMenu, HICON, HMENU, ICONINFO,
    IMAGE_ICON, LR_LOADFROMFILE, MF_SEPARATOR, MF_STRING, SB_LINEDOWN, SB_LINEUP, TPM_LEFTALIGN,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_SETREDRAW, WM_TIMER, WM_VSCROLL, WS_CHILD, WS_EX_CLIENTEDGE, WS_VISIBLE,
};

use crate::app::App;
use crate::win;
use crate::wnd::menu;
use crate::wnd::status;

/// The 8 built-in "Change Icon" choices: key → label. The order fixes the image
/// list indices.
pub const CATALOG: [(&str, &str); 8] = [
    ("folder", "Folder"),
    ("note", "Note"),
    ("star", "Star"),
    ("book", "Book"),
    ("flag", "Flag"),
    ("code", "Code"),
    ("bulb", "Idea"),
    ("warning", "Warning"),
];

/// Subclass id used for the drag handling on the tree.
const SUBCLASS_ID: usize = 0x4E4F4449; // 'NODI'

pub fn catalog() -> Vec<(&'static str, String)> {
    CATALOG
        .iter()
        .map(|(key, label)| (*key, (*label).to_owned()))
        .collect()
}

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

// --------------------------------------------------------------------- icons

/// Builds the shared image list from shell stock icons / `imageres.dll` and
/// attaches it to the tree. Every catalog entry always yields one image, so the
/// indices stay stable even when a shell icon is unavailable.
pub fn create_images(app: &mut App) {
    let list =
        unsafe { ImageList_Create(16, 16, ILC_COLOR32 | ILC_MASK, CATALOG.len() as i32 + 1, 4) };
    if list == 0 {
        app.icon_keys = CATALOG.iter().map(|(key, _)| *key).collect();
        return;
    }

    let fallback = stock_icon(SIID_APPLICATION);
    for (key, _) in CATALOG.iter() {
        let loaded = load_icon(key);
        let use_icon = if loaded.is_null() { fallback } else { loaded };
        if !use_icon.is_null() {
            // SAFETY: `list` is our image list, `use_icon` a live icon.
            unsafe {
                ImageList_ReplaceIcon(list, -1, use_icon);
            }
        }
        if !loaded.is_null() {
            // SAFETY: the image list copied the icon.
            unsafe {
                DestroyIcon(loaded);
            }
        }
    }
    if !fallback.is_null() {
        // SAFETY: the image list copied the icon.
        unsafe {
            DestroyIcon(fallback);
        }
    }

    // One more image for the permanent "Trash (N)" pseudo-item.
    let recycle = stock_icon(SIID_RECYCLER);
    let trash_image = if recycle.is_null() {
        0
    } else {
        // SAFETY: `list` is our image list and `recycle` a live icon.
        let index = unsafe { ImageList_ReplaceIcon(list, -1, recycle) };
        // SAFETY: the image list copied the icon.
        unsafe {
            DestroyIcon(recycle);
        }
        index.max(0)
    };
    app.trash_image = trash_image;

    app.icon_keys = CATALOG.iter().map(|(key, _)| *key).collect();
    app.images = list;
    if !app.tree_view.is_null() {
        // SAFETY: `tree_view` is our TreeView, `list` stays alive until destroy.
        unsafe {
            SendMessageW(app.tree_view, TVM_SETIMAGELIST, TVSIL_NORMAL as usize, list);
        }
    }
}

fn load_icon(key: &str) -> HICON {
    match key {
        "folder" => stock_icon(SIID_FOLDER),
        "note" => stock_icon(SIID_DOCNOASSOC),
        // No star/book/flag/code/bulb glyph exists as a stock icon; these come
        // from `imageres.dll` resource ids. `extract` falls back to a distinct
        // stock icon when the resource cannot be loaded.
        "star" => extract("imageres.dll", 204, SIID_DOCASSOC),
        "book" => extract("imageres.dll", 97, SIID_HELP),
        "flag" => extract("imageres.dll", 57, SIID_SHIELD),
        "code" => extract("imageres.dll", 76, SIID_SOFTWARE),
        "bulb" => extract("imageres.dll", 206, SIID_INFO),
        "warning" => extract("imageres.dll", 219, SIID_WARNING),
        _ => ptr::null_mut(),
    }
}

fn stock_icon(siid: SHSTOCKICONID) -> HICON {
    let mut info: SHSTOCKICONINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHSTOCKICONINFO>() as u32;
    // SAFETY: `info` is a valid, fully initialised struct for the call.
    let hr = unsafe { SHGetStockIconInfo(siid, SHGSI_ICON | SHGSI_SMALLICON, &mut info) };
    if hr >= 0 {
        info.hIcon
    } else {
        ptr::null_mut()
    }
}

/// Extracts icon resource `resource` (a resource id, so it is passed negated)
/// from `module`, falling back to a stock icon.
fn extract(module: &str, resource: i32, fallback: SHSTOCKICONID) -> HICON {
    let wide = win::wide(module);
    let mut large: HICON = ptr::null_mut();
    let mut small: HICON = ptr::null_mut();
    // SAFETY: the string buffer outlives the call; both out pointers are valid.
    let count = unsafe { ExtractIconExW(wide.as_ptr(), -resource, &mut large, &mut small, 1) };
    if count == 0 {
        return stock_icon(fallback);
    }
    if !small.is_null() {
        if !large.is_null() {
            // SAFETY: a handle `ExtractIconExW` gave us.
            unsafe {
                DestroyIcon(large);
            }
        }
        small
    } else {
        large
    }
}

/// Appends a user-chosen icon file (`.ico` or `.png`) to the shared image list
/// and to `icon_keys`. Returns false when the file cannot be used.
pub fn add_icon(app: &mut App, key: &'static str, path: &std::path::Path) -> bool {
    if app.images == 0 {
        return false;
    }
    let icon = icon_from_file(path);
    if icon.is_null() {
        return false;
    }
    // SAFETY: `app.images` is our image list; the list copies the icon.
    let index = unsafe { ImageList_ReplaceIcon(app.images, -1, icon) };
    // SAFETY: the list made its own copy.
    unsafe {
        DestroyIcon(icon);
    }
    if index < 0 {
        return false;
    }
    app.icon_keys.push(key);
    true
}

/// Loads an `.ico` with `LoadImage`, or decodes a `.png` (16–32 px) into an
/// icon. Small files only: this is a tree glyph, not note content.
fn icon_from_file(path: &std::path::Path) -> HICON {
    let is_png = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"));
    if !is_png {
        let wide = win::wide(&path.to_string_lossy());
        // SAFETY: the path buffer outlives the call; LoadImage copies the icon.
        let icon = unsafe {
            LoadImageW(
                ptr::null_mut(),
                wide.as_ptr(),
                IMAGE_ICON,
                16,
                16,
                LR_LOADFROMFILE,
            )
        };
        if !icon.is_null() {
            return icon;
        }
        return ptr::null_mut();
    }
    let Ok(bytes) = std::fs::read(path) else {
        return ptr::null_mut();
    };
    let Ok(image) = nodi_core::png::decode(&bytes) else {
        return ptr::null_mut();
    };
    if image.width == 0 || image.height == 0 || image.width > 256 || image.height > 256 {
        return ptr::null_mut();
    }
    icon_from_rgba(&image)
}

/// 16×16 BGRA icon from a decoded PNG (nearest-neighbour scale + centre).
fn icon_from_rgba(image: &nodi_core::png::Rgba) -> HICON {
    const SIZE: u32 = 16;
    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
    let scale = (image.width.max(image.height) as f32 / SIZE as f32).max(1.0);
    let drawn_w = (image.width as f32 / scale).round().max(1.0) as u32;
    let drawn_h = (image.height as f32 / scale).round().max(1.0) as u32;
    let offset_x = (SIZE - drawn_w) / 2;
    let offset_y = (SIZE - drawn_h) / 2;
    for y in 0..drawn_h {
        for x in 0..drawn_w {
            let source_x = (x as f32 * scale) as u32;
            let source_y = (y as f32 * scale) as u32;
            let Some((red, green, blue, alpha)) = image.pixel(source_x, source_y) else {
                continue;
            };
            let at = (((y + offset_y) * SIZE + x + offset_x) * 4) as usize;
            pixels[at] = blue;
            pixels[at + 1] = green;
            pixels[at + 2] = red;
            pixels[at + 3] = alpha;
        }
    }

    // SAFETY: all GDI calls below create objects this function owns and frees.
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE as i32,
                // Negative height: top-down rows, the order `pixels` uses.
                biHeight: -(SIZE as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD {
                rgbBlue: 0,
                rgbGreen: 0,
                rgbRed: 0,
                rgbReserved: 0,
            }],
        };
        let mut bits: *mut core::ffi::c_void = ptr::null_mut();
        let colour = CreateDIBSection(
            ptr::null_mut(),
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            ptr::null_mut(),
            0,
        );
        if colour.is_null() || bits.is_null() {
            return ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
        // A zero mask marks every pixel as opaque; the PNG alpha channel in
        // the colour bitmap does the real blending.
        let mask = CreateBitmap(SIZE as i32, SIZE as i32, 1, 1, ptr::null());
        let icon_info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: colour,
        };
        let icon = CreateIconIndirect(&icon_info);
        DeleteObject(colour);
        if !mask.is_null() {
            DeleteObject(mask);
        }
        icon
    }
}

// ------------------------------------------------------------------- rebuild

struct Pending {
    id: NoteId,
    title: String,
    kind: NodeKind,
    icon: Option<String>,
    has_children: bool,
}

/// Rebuilds the whole TreeView from the model, restoring selection, expansion
/// and icons.
pub fn rebuild(app: &mut App) {
    if app.tree_view.is_null() {
        return;
    }
    app.suppress_select += 1;
    // Keep the view where it was (a drop or rename must not jump to the
    // top), and paint once at the end instead of item by item.
    let first_visible = first_visible_note(app);
    // SAFETY: `tree_view` is our TreeView; TVI_ROOT deletes every item.
    unsafe {
        SendMessageW(app.tree_view, WM_SETREDRAW, 0, 0);
        SendMessageW(app.tree_view, TVM_DELETEITEM, TVI_ROOT as usize, 0);
    }
    app.tree_items.clear();
    app.item_for.clear();
    app.trash_item = 0;

    for info in child_infos(app, None) {
        insert_subtree(app, TVI_ROOT, info, 0);
    }
    insert_trash_item(app);

    if let Some(id) = app.selected.clone() {
        select_item(app, &id);
    }
    if let Some(item) = first_visible.and_then(|id| item_of(app, &id)) {
        // SAFETY: `item` is a live item of our TreeView.
        unsafe {
            SendMessageW(
                app.tree_view,
                TVM_SELECTITEM,
                TVGN_FIRSTVISIBLE as usize,
                item,
            );
        }
    }
    if let Some(id) = app.selected.clone() {
        if let Some(item) = item_of(app, &id) {
            // SAFETY: as above.
            unsafe {
                SendMessageW(app.tree_view, TVM_ENSUREVISIBLE, 0, item);
            }
        }
    }
    // SAFETY: as above; repaint everything once.
    unsafe {
        SendMessageW(app.tree_view, WM_SETREDRAW, 1, 0);
        InvalidateRect(app.tree_view, ptr::null(), 1);
    }
    app.suppress_select -= 1;
}

/// The note shown in the tree's first visible row.
fn first_visible_note(app: &App) -> Option<NoteId> {
    // SAFETY: plain query of our TreeView.
    let item = unsafe {
        SendMessageW(
            app.tree_view,
            TVM_GETNEXTITEM,
            TVGN_FIRSTVISIBLE as usize,
            0,
        )
    };
    item_note(app, item)
}

/// Adds the permanent "Trash (N)" pseudo-item at the bottom of the tree. It is
/// not a note: it has no entry in `tree_items`, so every node command ignores
/// it, and dragging it is impossible.
fn insert_trash_item(app: &mut App) {
    let count = app.trashed_roots().len();
    let label = format!("Trash ({count})");
    let mut text = win::wide(&label);
    let item = TVITEMW {
        mask: TVIF_TEXT | TVIF_IMAGE | TVIF_SELECTEDIMAGE,
        hItem: 0,
        state: 0,
        stateMask: 0,
        pszText: text.as_mut_ptr(),
        cchTextMax: text.len() as i32,
        iImage: app.trash_image,
        iSelectedImage: app.trash_image,
        cChildren: 0,
        lParam: 0,
    };
    let insert = TVINSERTSTRUCTW {
        hParent: TVI_ROOT,
        hInsertAfter: TVI_LAST,
        Anonymous: TVINSERTSTRUCTW_0 { item },
    };
    // SAFETY: `insert` and `text` live for the duration of the call.
    let handle = unsafe {
        SendMessageW(
            app.tree_view,
            TVM_INSERTITEMW,
            0,
            &insert as *const TVINSERTSTRUCTW as isize,
        )
    } as isize;
    app.trash_item = handle;
}

/// Text of the permanent Trash pseudo-item (empty when it does not exist).
pub fn trash_label(app: &App) -> String {
    if app.trash_item == 0 || app.tree_view.is_null() {
        return String::new();
    }
    let mut buffer = [0u16; 64];
    let mut item = TVITEMW {
        mask: TVIF_TEXT,
        hItem: app.trash_item,
        state: 0,
        stateMask: 0,
        pszText: buffer.as_mut_ptr(),
        cchTextMax: buffer.len() as i32,
        iImage: 0,
        iSelectedImage: 0,
        cChildren: 0,
        lParam: 0,
    };
    // SAFETY: `item` and `buffer` outlive the call; `tree_view` is ours.
    let ok = unsafe {
        SendMessageW(
            app.tree_view,
            TVM_GETITEMW,
            0,
            &mut item as *mut TVITEMW as isize,
        )
    };
    if ok == 0 {
        return String::new();
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// Title shown in the tree: a locked node carries a padlock.
pub fn display_title(node: &nodi_core::Node) -> String {
    if node.locked {
        format!("{} 🔒", node.title)
    } else {
        node.title.clone()
    }
}

fn child_infos(app: &App, parent: Option<&NoteId>) -> Vec<Pending> {
    let settings = &app.settings;
    app.tree
        .sorted_children(
            parent,
            settings.tree_sort,
            settings.tree_sort_desc,
            settings.folders_first,
        )
        .into_iter()
        .map(|node| Pending {
            id: node.id.clone(),
            title: display_title(node),
            kind: node.kind,
            icon: node.icon.clone(),
            has_children: !app.tree.children(Some(&node.id)).is_empty(),
        })
        .collect()
}

fn insert_subtree(app: &mut App, parent_item: isize, info: Pending, depth: usize) {
    if depth > 512 {
        return;
    }
    let handle = insert_item(app, parent_item, &info);
    if handle == 0 {
        return;
    }
    for child in child_infos(app, Some(&info.id)) {
        insert_subtree(app, handle, child, depth + 1);
    }
    if app.expanded.contains(&info.id) {
        // SAFETY: `handle` is a live item of our TreeView.
        unsafe {
            SendMessageW(app.tree_view, TVM_EXPAND, TVE_EXPAND as usize, handle);
        }
    }
}

fn insert_item(app: &mut App, parent_item: isize, info: &Pending) -> isize {
    let mut text = win::wide(&info.title);
    let image = icon_index_for(app, info.kind, info.icon.as_deref());
    let item = TVITEMW {
        mask: TVIF_TEXT | TVIF_IMAGE | TVIF_SELECTEDIMAGE,
        hItem: 0,
        state: 0,
        stateMask: 0,
        pszText: text.as_mut_ptr(),
        cchTextMax: text.len() as i32,
        iImage: image,
        iSelectedImage: image,
        cChildren: i32::from(info.has_children),
        lParam: 0,
    };
    // The Trash pseudo-item stays the last top-level row.
    let after = if parent_item == TVI_ROOT && app.trash_item != 0 {
        // SAFETY: plain query of our TreeView.
        let previous = unsafe {
            SendMessageW(
                app.tree_view,
                TVM_GETNEXTITEM,
                TVGN_PREVIOUS as usize,
                app.trash_item,
            )
        };
        if previous == 0 {
            TVI_FIRST
        } else {
            previous
        }
    } else {
        TVI_LAST
    };
    let insert = TVINSERTSTRUCTW {
        hParent: parent_item,
        hInsertAfter: after,
        Anonymous: TVINSERTSTRUCTW_0 { item },
    };
    // SAFETY: `insert` and `text` live for the duration of the call.
    let handle = unsafe {
        SendMessageW(
            app.tree_view,
            TVM_INSERTITEMW,
            0,
            &insert as *const TVINSERTSTRUCTW as isize,
        )
    } as isize;
    if handle != 0 {
        app.tree_items.insert(handle, info.id.clone());
        app.item_for.insert(info.id.clone(), handle);
    }
    handle
}

fn icon_index_for(app: &App, kind: NodeKind, icon: Option<&str>) -> i32 {
    if let Some(key) = icon {
        if let Some(position) = app.icon_keys.iter().position(|candidate| *candidate == key) {
            return position as i32;
        }
    }
    let wanted = match kind {
        NodeKind::Folder => "folder",
        NodeKind::Note => "note",
    };
    app.icon_keys
        .iter()
        .position(|candidate| *candidate == wanted)
        .unwrap_or(0) as i32
}

// ------------------------------------------------------------------ item ops

fn item_of(app: &App, id: &NoteId) -> Option<isize> {
    app.item_for.get(id).copied()
}

/// Adds one freshly created node (a last child) without rebuilding the tree.
/// Returns false when its parent has no item, so the caller rebuilds.
pub fn add_node(app: &mut App, id: &NoteId) -> bool {
    // A sorted tree puts the node somewhere in the middle: rebuild.
    if app.settings.tree_sort != nodi_core::tree::SortOrder::Manual {
        return false;
    }
    let Some(node) = app.tree.get(id) else {
        return false;
    };
    let parent_item = match &node.parent {
        Some(parent) => match item_of(app, parent) {
            Some(item) => item,
            None => return false,
        },
        None => TVI_ROOT,
    };
    let info = Pending {
        id: id.clone(),
        title: node.title.clone(),
        kind: node.kind,
        icon: node.icon.clone(),
        has_children: false,
    };
    let handle = insert_item(app, parent_item, &info);
    if handle == 0 {
        return false;
    }
    if parent_item != TVI_ROOT {
        // The parent now has children: give it its expand button.
        let mut update = TVITEMW {
            mask: TVIF_CHILDREN,
            hItem: parent_item,
            cChildren: 1,
            ..TVITEMW::default()
        };
        // SAFETY: `update` lives for the call; `parent_item` is ours.
        unsafe {
            SendMessageW(
                app.tree_view,
                TVM_SETITEMW,
                0,
                &mut update as *mut TVITEMW as isize,
            );
        }
    }
    select_item(app, id);
    true
}

/// Selects an item without letting the change open a note (programmatic).
pub fn select_item(app: &mut App, id: &NoteId) -> bool {
    let Some(item) = item_of(app, id) else {
        return false;
    };
    app.suppress_select += 1;
    // SAFETY: `item` is a live item of our TreeView.
    unsafe {
        SendMessageW(app.tree_view, TVM_SELECTITEM, TVGN_CARET as usize, item);
    }
    app.suppress_select -= 1;
    app.selected = Some(id.clone());
    true
}

pub fn set_item_text(app: &App, id: &NoteId, title: &str) {
    let Some(item) = item_of(app, id) else {
        return;
    };
    let mut text = win::wide(title);
    let mut update = TVITEMW {
        mask: TVIF_TEXT,
        hItem: item,
        pszText: text.as_mut_ptr(),
        cchTextMax: text.len() as i32,
        ..TVITEMW::default()
    };
    // SAFETY: `update`/`text` live for the duration of the call.
    unsafe {
        SendMessageW(
            app.tree_view,
            TVM_SETITEMW,
            0,
            &mut update as *mut TVITEMW as isize,
        );
    }
}

pub fn set_item_icon(app: &App, id: &NoteId, image: i32) {
    let Some(item) = item_of(app, id) else {
        return;
    };
    let mut update = TVITEMW {
        mask: TVIF_IMAGE | TVIF_SELECTEDIMAGE,
        hItem: item,
        iImage: image,
        iSelectedImage: image,
        ..TVITEMW::default()
    };
    // SAFETY: `update` lives for the duration of the call.
    unsafe {
        SendMessageW(
            app.tree_view,
            TVM_SETITEMW,
            0,
            &mut update as *mut TVITEMW as isize,
        );
    }
}

pub fn expand_item(app: &App, id: &NoteId) {
    let Some(item) = item_of(app, id) else {
        return;
    };
    // SAFETY: `item` is a live item of our TreeView.
    unsafe {
        SendMessageW(app.tree_view, TVM_EXPAND, TVE_EXPAND as usize, item);
    }
}

/// Starts the inline label edit for the selected (or context) node.
pub fn begin_rename(app: &mut App) {
    let Some(id) = app.context_note.clone().or_else(|| app.selected.clone()) else {
        return;
    };
    let Some(item) = item_of(app, &id) else {
        return;
    };
    if app.tree_view.is_null() {
        return;
    }
    // `TVM_EDITLABELW` only starts the edit when the TreeView itself has the
    // focus; without this the edit box appears but every keystroke goes to the
    // tree (and beeps). Rename arrives from the context menu, F2 and the Notes
    // menu, where focus was still on the editor.
    // SAFETY: `item` is a live item of our TreeView.
    unsafe {
        SetFocus(app.tree_view);
        SendMessageW(app.tree_view, TVM_EDITLABELW, 0, item);
    }
}

/// The item handle under the caret (0 when the tree has no selection).
fn caret_item(app: &App) -> isize {
    if app.tree_view.is_null() {
        return 0;
    }
    // SAFETY: `TVGN_CARET` only reads the tree's selection.
    unsafe { SendMessageW(app.tree_view, TVM_GETNEXTITEM, TVGN_CARET as usize, 0) as isize }
}

/// True while a node title is being edited inline.
fn label_edit_open(app: &App) -> bool {
    if app.tree_view.is_null() {
        return false;
    }
    // SAFETY: `TVM_GETEDITCONTROL` only reads the tree state.
    unsafe { SendMessageW(app.tree_view, TVM_GETEDITCONTROL, 0, 0) != 0 }
}

/// True when the tree's caret sits on the "Trash (N)" pseudo-item.
pub fn trash_focused(app: &App) -> bool {
    app.trash_item != 0 && caret_item(app) == app.trash_item
}

// ------------------------------------------------------------------ hit test

fn item_note(app: &App, item: isize) -> Option<NoteId> {
    if item == 0 {
        return None;
    }
    app.tree_items.get(&item).cloned()
}

fn is_note(app: &App, id: &NoteId) -> bool {
    app.tree
        .get(id)
        .is_some_and(|node| node.kind == NodeKind::Note && !node.trashed)
}

/// Item whose row contains `client`, anywhere in the row: label, icon,
/// indent, expand button or the empty space right of the label (0 below the
/// last row). Dragging must work across the whole row, not only on the text.
fn hit_row(app: &App, client: POINT) -> isize {
    let mut info = TVHITTESTINFO {
        pt: client,
        flags: 0,
        hItem: 0,
    };
    // SAFETY: `info` lives for the duration of the call.
    let item = unsafe {
        SendMessageW(
            app.tree_view,
            TVM_HITTEST,
            0,
            &mut info as *mut TVHITTESTINFO as isize,
        )
    } as isize;
    let row = TVHT_ONITEM | TVHT_ONITEMRIGHT | TVHT_ONITEMINDENT | TVHT_ONITEMBUTTON;
    if item != 0 && info.flags & row != 0 {
        item
    } else {
        0
    }
}

/// Client rectangle of `item`'s whole row.
fn row_rect(app: &App, item: isize) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: TVM_GETITEMRECT reads the item handle from the start of the
    // RECT and overwrites it with the result; wParam FALSE = the whole row.
    let ok = unsafe {
        *(&mut rect as *mut RECT as *mut isize) = item;
        SendMessageW(
            app.tree_view,
            TVM_GETITEMRECT,
            0,
            &mut rect as *mut RECT as isize,
        )
    };
    (ok != 0).then_some(rect)
}

// --------------------------------------------------------------- notifications

/// Every row in display order, as far as it is expanded.
pub fn visible_ids(app: &App) -> Vec<NoteId> {
    let mut ids = Vec::new();
    // SAFETY: plain queries of our TreeView.
    let mut item = unsafe { SendMessageW(app.tree_view, TVM_GETNEXTITEM, TVGN_ROOT as usize, 0) };
    let mut guard = 0;
    while item != 0 && guard < 100_000 {
        if let Some(id) = item_note(app, item) {
            ids.push(id);
        }
        // SAFETY: as above.
        item = unsafe {
            SendMessageW(
                app.tree_view,
                TVM_GETNEXTITEM,
                TVGN_NEXTVISIBLE as usize,
                item,
            )
        };
        guard += 1;
    }
    ids
}

/// Repaints the tree (multi-selection changed).
pub fn redraw(app: &App) {
    if !app.tree_view.is_null() {
        // SAFETY: our own window.
        unsafe {
            InvalidateRect(app.tree_view, ptr::null(), 1);
        }
    }
}

/// Notifications whose return value matters: the multi-selection colours
/// (`NM_CUSTOMDRAW`) and refusing to rename a locked node.
pub fn notify_result(app: &mut App, header: &NMHDR, lparam: LPARAM) -> Option<LRESULT> {
    if header.hwndFrom != app.tree_view || app.tree_view.is_null() {
        return None;
    }
    match header.code {
        NM_CUSTOMDRAW => {
            let draw = lparam as *mut NMTVCUSTOMDRAW;
            // SAFETY: the TreeView sent an NMTVCUSTOMDRAW for this code.
            unsafe {
                match (*draw).nmcd.dwDrawStage {
                    CDDS_PREPAINT => {
                        if app.multi.len() > 1 {
                            return Some(CDRF_NOTIFYITEMDRAW as LRESULT);
                        }
                        None
                    }
                    CDDS_ITEMPREPAINT => {
                        let item = (*draw).nmcd.dwItemSpec as isize;
                        let picked = item_note(app, item).is_some_and(|id| app.multi.contains(&id));
                        if picked {
                            (*draw).clrTextBk = app.palette.accent;
                            (*draw).clrText = app.palette.editor_text;
                            return Some(CDRF_NEWFONT as LRESULT);
                        }
                        Some(CDRF_DODEFAULT as LRESULT)
                    }
                    _ => None,
                }
            }
        }
        TVN_BEGINLABELEDITW => {
            let info = header as *const NMHDR as *const NMTVDISPINFOW;
            // SAFETY: the TreeView sent an NMTVDISPINFOW for this code.
            let item = unsafe { (*info).item.hItem };
            let id = item_note(app, item)?;
            if app.tree.is_locked(&id) {
                app.check_unlocked(&id);
                return Some(1);
            }
            None
        }
        _ => None,
    }
}

/// Handles a `WM_NOTIFY` that came from the TreeView. Returns true when the
/// notification belonged to the tree.
pub fn handle_notify(app: &mut App, header: &NMHDR) -> bool {
    if header.hwndFrom != app.tree_view {
        return false;
    }
    match header.code {
        TVN_SELCHANGEDW => {
            let info = header as *const NMHDR as *const NMTREEVIEWW;
            // SAFETY: the TreeView sent an NMTREEVIEWW for this code.
            let (item, action) = unsafe { ((*info).itemNew.hItem, (*info).action) };
            if app.trash_item != 0 && item == app.trash_item {
                // The pseudo-item is not a note: nothing may be renamed,
                // trashed or duplicated through it.
                app.selected = None;
                const TVC_BYMOUSE: u32 = 1;
                if app.suppress_select == 0 && action == TVC_BYMOUSE {
                    // Post instead of running the modal Trash window inside a
                    // notification handler.
                    // SAFETY: posting to our own frame window.
                    unsafe {
                        PostMessageW(
                            app.hwnd,
                            windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND,
                            menu::IDM_NOTES_TRASH_VIEW as usize,
                            0,
                        );
                    }
                }
                return true;
            }
            if let Some(id) = item_note(app, item) {
                app.selected = Some(id.clone());
                // A click opens the note; arrow keys only move the selection
                // (Enter opens), so browsing does not spawn a tab per note.
                const TVC_BYMOUSE: u32 = 1;
                if app.suppress_select == 0 && action == TVC_BYMOUSE && is_note(app, &id) {
                    app.open_note(id);
                }
            }
        }
        TVN_ITEMEXPANDEDW => {
            let info = header as *const NMHDR as *const NMTREEVIEWW;
            // SAFETY: the TreeView sent an NMTREEVIEWW for this code.
            let (item, state) = unsafe { ((*info).itemNew.hItem, (*info).itemNew.state) };
            if let Some(id) = item_note(app, item) {
                if state & TVIS_EXPANDED != 0 {
                    app.expanded.insert(id);
                } else {
                    app.expanded.remove(&id);
                }
                app.save_session_now();
            }
        }
        TVN_BEGINLABELEDITW => return true,
        TVN_ENDLABELEDITW => {
            let info = header as *const NMHDR as *const NMTVDISPINFOW;
            // SAFETY: the TreeView sent an NMTVDISPINFOW for this code.
            let (item, text) = unsafe { ((*info).item.hItem, (*info).item.pszText) };
            if text.is_null() {
                return true;
            }
            if let Some(id) = item_note(app, item) {
                // SAFETY: `text` is a NUL-terminated buffer owned by the control.
                let title = unsafe { read_wide(text) };
                if !title.trim().is_empty() {
                    app.rename_node(id, title);
                }
            }
        }
        TVN_BEGINDRAGW => {
            let info = header as *const NMHDR as *const NMTREEVIEWW;
            // SAFETY: the TreeView sent an NMTREEVIEWW for this code.
            let (item, point) = unsafe { ((*info).itemNew.hItem, (*info).ptDrag) };
            begin_drag(app, item, point);
        }
        TVN_KEYDOWN => {
            let info = header as *const NMHDR as *const NMTVKEYDOWN;
            // SAFETY: the TreeView sent an NMTVKEYDOWN for this code.
            let key = unsafe { (*info).wVKey };
            match key {
                VK_RETURN => {
                    if trash_focused(app) {
                        // Enter on the pseudo-item opens the Trash window.
                        // SAFETY: posting to our own frame window.
                        unsafe {
                            PostMessageW(
                                app.hwnd,
                                windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND,
                                menu::IDM_NOTES_TRASH_VIEW as usize,
                                0,
                            );
                        }
                    } else if let Some(id) = app.selected.clone() {
                        if is_note(app, &id) {
                            app.open_note(id);
                        }
                    }
                }
                VK_DELETE => {
                    if !trash_focused(app) {
                        app.trash_selection();
                    }
                }
                VK_ESCAPE => app.clear_multi(),
                VK_APPS => show_context_menu(app, None),
                _ => {}
            }
        }
        NM_RCLICK => {
            MOUSE_MENU.with(|shown| shown.set(true));
            show_context_menu(app, Some(cursor_pos()));
        }
        NM_DBLCLK => {
            if let Some(id) = app.context_note.clone().or_else(|| app.selected.clone()) {
                if is_note(app, &id) {
                    app.open_note(id);
                }
            }
        }
        _ => return true,
    }
    true
}

// -------------------------------------------------------------- drag & drop

pub fn install_subclass(app: &App) {
    if app.tree_view.is_null() {
        return;
    }
    // SAFETY: our TreeView; the frame handle travels as the reference data.
    unsafe {
        SetWindowSubclass(
            app.tree_view,
            Some(tree_subclass),
            SUBCLASS_ID,
            app.hwnd as usize,
        );
    }
}

/// Timer that keeps auto-scroll and hover-expand alive while the mouse is
/// still (no `WM_MOUSEMOVE` arrives then).
const DRAG_TIMER: usize = 0x4E44; // 'ND'
/// How long a collapsed node must be hovered to expand during a drag.
const HOVER_EXPAND: u128 = 700;
/// Distance from the top/bottom edge that starts auto-scrolling.
const SCROLL_MARGIN: i32 = 24;

unsafe extern "system" fn tree_subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    refdata: usize,
) -> LRESULT {
    let frame = refdata as HWND;
    match msg {
        WM_LBUTTONDOWN => {
            if let Some(app) = crate::wnd::app_mut(frame) {
                let point = POINT {
                    x: win::x_from_lparam(lparam),
                    y: win::y_from_lparam(lparam),
                };
                let ctrl = wparam & 0x0008 != 0; // MK_CONTROL
                let shift = wparam & 0x0004 != 0; // MK_SHIFT
                let item = hit_row(app, point);
                let id = item_note(app, item);
                if ctrl || shift {
                    // Ctrl+click / Shift+click build a multi-selection; the
                    // caret (and the open note) stay where they are.
                    if let Some(id) = id {
                        SetFocus(hwnd);
                        if ctrl {
                            app.toggle_multi(id);
                        } else {
                            app.range_multi(id);
                        }
                    }
                    return 0;
                }
                let in_multi = id.as_ref().is_some_and(|id| app.multi.contains(id));
                if !in_multi {
                    app.clear_multi();
                    app.multi_anchor = id;
                    return DefSubclassProc(hwnd, msg, wparam, lparam);
                }
                // A press on a selected row may start dragging the whole
                // selection; a plain click (no drag) selects just that row.
                let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                if let Some(app) = crate::wnd::app_mut(frame) {
                    if !app.dragging {
                        app.clear_multi();
                    }
                }
                return result;
            }
        }
        WM_MOUSEMOVE => {
            if let Some(app) = crate::wnd::app_mut(frame) {
                if app.dragging {
                    on_drag_move(app, lparam);
                    return 0;
                }
            }
        }
        WM_LBUTTONUP => {
            if let Some(app) = crate::wnd::app_mut(frame) {
                if app.dragging {
                    on_drop(app, lparam);
                    return 0;
                }
            }
        }
        WM_KEYDOWN => {
            if let Some(app) = crate::wnd::app_mut(frame) {
                if app.dragging && wparam == VK_ESCAPE as usize {
                    finish_drag(app);
                    status::set_message(app, "Move cancelled");
                    return 0;
                }
            }
        }
        WM_TIMER if wparam == DRAG_TIMER => {
            if let Some(app) = crate::wnd::app_mut(frame).filter(|app| app.dragging) {
                drag_tick(app);
                return 0;
            }
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

fn begin_drag(app: &mut App, item: isize, point: POINT) {
    let Some(id) = item_note(app, item) else {
        return;
    };
    let Some(node) = app.tree.get(&id) else {
        return;
    };
    if app.images == 0 {
        return;
    }
    let index = icon_index_for(app, node.kind, node.icon.as_deref());
    app.drag_source = Some(id);
    app.dragging = true;
    app.drag_hover = None;
    app.drag_hover_since = None;
    app.drag_point = Some((point.x, point.y));
    // SAFETY: `images`/`tree_view` belong to us; the drag lives until drop.
    unsafe {
        ImageList_BeginDrag(app.images, index, 8, 8);
        ImageList_DragEnter(app.tree_view, point.x, point.y);
        SetCapture(app.tree_view);
        SetTimer(app.tree_view, DRAG_TIMER, 100, None);
    }
}

fn on_drag_move(app: &mut App, lparam: LPARAM) {
    let point = POINT {
        x: win::x_from_lparam(lparam),
        y: win::y_from_lparam(lparam),
    };
    // SAFETY: a drag image is in progress; coordinates are tree client coords.
    unsafe {
        ImageList_DragMove(point.x, point.y);
    }
    app.drag_point = Some((point.x, point.y));
    auto_scroll(app, point.y);
    update_hover(app, point);
}

/// True while `point` is over the tree's "Trash" pseudo-item.
fn over_trash(app: &App, point: POINT) -> bool {
    app.trash_item != 0 && hit_row(app, point) == app.trash_item
}

/// Item under `client` plus the drop zone for its row: the top quarter means
/// "before", the bottom quarter "after", the middle "inside".
fn drop_zone(app: &App, client: POINT) -> Option<(NoteId, Position)> {
    let item = hit_row(app, client);
    if item == 0 || item == app.trash_item {
        return None;
    }
    let id = item_note(app, item)?;
    let rect = row_rect(app, item)?;
    let band = ((rect.bottom - rect.top) / 4).max(1);
    let position = if client.y < rect.top + band {
        Position::Before
    } else if client.y >= rect.bottom - band {
        Position::After
    } else {
        Position::Inside
    };
    Some((id, position))
}

/// Draws the drop feedback: an insertion line for before/after, the
/// `TVGN_DROPHILITE` highlight for "inside".
fn show_drop(app: &App, hover: Option<(NoteId, Position)>) {
    if app.tree_view.is_null() {
        return;
    }
    // The drag image is drawn over the tree with XOR; hide it while the marks
    // repaint the rows below it, or it leaves trails behind.
    if app.dragging {
        // SAFETY: a drag image is in progress.
        unsafe {
            ImageList_DragShowNolock(0);
        }
    }
    let _restore = DragImageBack(app.dragging);
    // SAFETY: plain messages to our TreeView; a zero item clears each mark.
    unsafe {
        SendMessageW(app.tree_view, TVM_SETINSERTMARK, 0, 0);
        SendMessageW(app.tree_view, TVM_SELECTITEM, TVGN_DROPHILITE as usize, 0);
        if let Some((id, position)) = hover {
            if let Some(item) = item_of(app, &id) {
                match position {
                    Position::Before => {
                        SendMessageW(app.tree_view, TVM_SETINSERTMARK, 0, item);
                    }
                    Position::After => {
                        SendMessageW(app.tree_view, TVM_SETINSERTMARK, 1, item);
                    }
                    Position::Inside => {
                        SendMessageW(
                            app.tree_view,
                            TVM_SELECTITEM,
                            TVGN_DROPHILITE as usize,
                            item,
                        );
                    }
                }
            }
        }
    }
}

/// Shows the drag image again (and repaints the tree under it first) when
/// dropped.
struct DragImageBack(bool);

impl Drop for DragImageBack {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: matching the DragShowNolock(0) above.
            unsafe {
                ImageList_DragShowNolock(1);
            }
        }
    }
}

fn update_hover(app: &mut App, point: POINT) {
    let hover = drop_zone(app, point);
    if hover == app.drag_hover {
        return;
    }
    app.drag_hover = hover;
    app.drag_hover_since = Some(std::time::Instant::now());
    let hover = app.drag_hover.clone();
    show_drop(app, hover);
}

/// Scrolls the tree when the drag cursor is near the top or bottom edge.
fn auto_scroll(app: &App, y: i32) {
    let mut rect = RECT::default();
    // SAFETY: `rect` is a valid out pointer; `tree_view` is ours.
    unsafe {
        GetClientRect(app.tree_view, &mut rect);
    }
    let (direction, distance) = if y < rect.top + SCROLL_MARGIN {
        (SB_LINEUP, rect.top + SCROLL_MARGIN - y)
    } else if y > rect.bottom - SCROLL_MARGIN {
        (SB_LINEDOWN, y - (rect.bottom - SCROLL_MARGIN))
    } else {
        return;
    };
    let lines = (distance / 8).clamp(1, 4);
    // SAFETY: drag image calls match an active drag; scroll messages go to
    // our own TreeView.
    unsafe {
        if app.dragging {
            ImageList_DragShowNolock(0);
        }
        for _ in 0..lines {
            SendMessageW(app.tree_view, WM_VSCROLL, direction as usize, 0);
        }
        if app.dragging {
            UpdateWindow(app.tree_view);
            ImageList_DragShowNolock(1);
        }
    }
}

/// Timer tick during a drag: auto-scroll, refresh the indicator and expand a
/// collapsed node that has been hovered for `HOVER_EXPAND` ms.
fn drag_tick(app: &mut App) {
    let Some((x, y)) = app.drag_point else {
        return;
    };
    auto_scroll(app, y);
    update_hover(app, POINT { x, y });
    let Some((id, Position::Inside)) = app.drag_hover.clone() else {
        return;
    };
    let hovered = app
        .drag_hover_since
        .is_some_and(|since| since.elapsed().as_millis() >= HOVER_EXPAND);
    if hovered && !app.expanded.contains(&id) && !app.tree.children(Some(&id)).is_empty() {
        app.drag_hover_since = Some(std::time::Instant::now());
        app.expand(Some(id));
    }
}

/// Ends the drag (drop or Esc) and clears every visual mark.
fn finish_drag(app: &mut App) {
    // SAFETY: matching the BeginDrag/DragEnter/SetCapture/SetTimer calls; a
    // missing capture or timer is harmless.
    unsafe {
        ImageList_DragLeave(app.tree_view);
        ImageList_EndDrag();
        ReleaseCapture();
        KillTimer(app.tree_view, DRAG_TIMER);
    }
    show_drop(app, None);
    app.dragging = false;
    app.drag_source = None;
    app.drag_hover = None;
    app.drag_hover_since = None;
    app.drag_point = None;
}

fn on_drop(app: &mut App, lparam: LPARAM) {
    let point = POINT {
        x: win::x_from_lparam(lparam),
        y: win::y_from_lparam(lparam),
    };
    let hover = drop_zone(app, point);
    let trash = over_trash(app, point);
    let source = app.drag_source.clone();
    finish_drag(app);
    if trash {
        return;
    }
    if let Some(source) = source {
        let group = app.multi.len() > 1 && app.multi.contains(&source);
        match hover {
            Some((target, position)) if target != source => {
                // A refused move is reported in the status bar.
                let _ = app.move_selection_relative(source, target, position);
            }
            Some(_) => {}
            // Dropped on empty space: move to the top level.
            None => {
                if !group {
                    app.multi.clear();
                    app.context_note = Some(source.clone());
                    app.selected = Some(source);
                }
                app.move_selection_into(None);
                app.context_note = None;
            }
        }
    }
}

// -------------------------------------------------------------- context menu

thread_local! {
    /// Set when `NM_RCLICK` has just shown the menu. Windows sends a
    /// `WM_CONTEXTMENU` for the same right-click after the tree finishes the
    /// button message (i.e. after our menu closes, with the cursor over the
    /// item we chose), which would open a second menu at that spot.
    static MOUSE_MENU: Cell<bool> = const { Cell::new(false) };
}

/// True when the menu for this right-click was already shown; clears the mark.
pub fn take_mouse_menu() -> bool {
    MOUSE_MENU.with(|shown| shown.replace(false))
}

/// Shows the node context menu. `screen` is the click position in screen
/// coordinates, or `None` for a keyboard invocation (under the selected item).
pub fn show_context_menu(app: &mut App, screen: Option<POINT>) {
    if app.tree_view.is_null() {
        return;
    }
    // While a title is being edited, a right-click must not pop up the menu
    // for the empty area (the label edit owns the input).
    if label_edit_open(app) {
        return;
    }

    let mut on_trash = false;
    let target = match screen {
        Some(point) => {
            let mut client = point;
            // SAFETY: the tree window is alive.
            unsafe {
                ScreenToClient(app.tree_view, &mut client);
            }
            // The whole row counts, not only the label.
            let item = hit_row(app, client);
            if app.trash_item != 0 && item == app.trash_item {
                on_trash = true;
                app.selected = None;
                None
            } else {
                match item_note(app, item) {
                    Some(id) => {
                        // Right-click outside the multi-selection: the
                        // menu is for that one row.
                        if !app.multi.contains(&id) {
                            app.clear_multi();
                        }
                        select_item(app, &id);
                        Some(id)
                    }
                    None => {
                        app.selected = None;
                        None
                    }
                }
            }
        }
        None => {
            if trash_focused(app) {
                on_trash = true;
                None
            } else {
                let id = app.selected.clone();
                if let Some(id) = &id {
                    select_item(app, id);
                }
                id
            }
        }
    };

    let position = screen
        .or_else(|| item_screen_pos(app, target.as_ref()))
        .unwrap_or(POINT { x: 0, y: 0 });

    app.context_note = target;
    let menu = if on_trash {
        build_trash_menu()
    } else if app.multi.len() > 1 {
        build_multi_menu(app)
    } else {
        build_menu(app)
    };
    if menu.is_null() {
        app.context_note = None;
        return;
    }
    // SAFETY: `menu` is a popup we own; `hwnd` is our frame window.
    let command = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN,
            position.x,
            position.y,
            0,
            app.hwnd,
            ptr::null(),
        )
    };
    // SAFETY: the popup is no longer needed.
    unsafe {
        DestroyMenu(menu);
    }
    if command != 0 {
        crate::app::context_command(app, command as u16);
    } else {
        app.context_note = None;
    }
}

fn build_trash_menu() -> HMENU {
    // SAFETY: every call below creates/edits a menu we own.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return menu;
        }
        append(menu, menu::IDM_NOTES_TRASH_VIEW, "Open Trash");
        append(menu, menu::IDM_NOTES_TRASH_EMPTY, "Empty Trash...");
        SetMenuDefaultItem(menu, menu::IDM_NOTES_TRASH_VIEW as u32, 0);
        menu
    }
}

/// Menu for several selected rows: only commands that make sense for all.
fn build_multi_menu(app: &App) -> HMENU {
    // SAFETY: every call below creates/edits a menu we own.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return menu;
        }
        let title = win::wide(&format!("{} items selected", app.multi.len()));
        AppendMenuW(
            menu,
            MF_STRING | windows_sys::Win32::UI::WindowsAndMessaging::MF_GRAYED,
            0,
            title.as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        append(menu, menu::IDM_NOTES_MOVE_TO, "Move to...");
        append(menu, menu::IDM_NOTES_MOVE_ROOT, "Move to Top Level");
        append(menu, menu::IDM_NOTES_EXPORT, "Export...");
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        if app.selection_locked() {
            append(menu, menu::IDM_NOTES_UNLOCK, "Unlock");
        } else {
            append(menu, menu::IDM_NOTES_LOCK, "Lock");
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        append(menu, menu::IDM_NOTES_TRASH, "Move to Trash");
        menu
    }
}

/// The lock / export / import part shared by note and folder menus.
unsafe fn append_common(menu: HMENU, app: &App, folder_like: bool) {
    AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    append(menu, menu::IDM_NOTES_EXPORT, "Export...");
    if folder_like {
        append(menu, menu::IDM_NOTES_IMPORT_HERE, "Import Files Here...");
        append(
            menu,
            menu::IDM_NOTES_IMPORT_FOLDER_HERE,
            "Import Folder Here...",
        );
    }
    let source = app
        .context_note
        .as_ref()
        .and_then(|id| app.import_source(id));
    if source.is_some() {
        append(menu, menu::IDM_NOTES_SHOW_ORIGINAL, "Show Original File");
        append(
            menu,
            menu::IDM_NOTES_SAVE_BACK,
            "Save Back to Original File...",
        );
    }
    let locked = app
        .context_note
        .as_ref()
        .and_then(|id| app.tree.get(id))
        .is_some_and(|node| node.locked);
    if locked {
        append(menu, menu::IDM_NOTES_UNLOCK, "Unlock");
    } else {
        append(menu, menu::IDM_NOTES_LOCK, "Lock");
    }
}

fn build_menu(app: &App) -> HMENU {
    // SAFETY: every call below creates/edits a menu we own.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return menu;
        }
        let kind = app
            .context_note
            .as_ref()
            .and_then(|id| app.tree.get(id))
            .map(|node| node.kind);
        match kind {
            None => {
                append(menu, menu::IDM_NOTES_NEW, "New Note");
                append(menu, menu::IDM_NOTES_NEW_FOLDER, "New Folder");
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                append(menu, menu::IDM_VIEW_EXPAND_ALL, "Expand All");
                append(menu, menu::IDM_VIEW_COLLAPSE_ALL, "Collapse All");
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                append(menu, menu::IDM_NOTES_IMPORT_HERE, "Import Files...");
                append(menu, menu::IDM_NOTES_IMPORT_FOLDER_HERE, "Import Folder...");
                append(menu, menu::IDM_FILE_EXPORT_ALL, "Export All Notes...");
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                append(
                    menu,
                    menu::IDM_SEARCH_ALL_NOTES,
                    "Find Notes...\tCtrl+Shift+F",
                );
                append(menu, menu::IDM_VIEW_REFRESH, "Refresh\tF5");
            }
            Some(NodeKind::Note) => {
                append(menu, menu::IDM_NOTES_OPEN, "Open");
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                append(menu, menu::IDM_NOTES_NEW_CHILD, "New Child Note");
                append(menu, menu::IDM_NOTES_NEW_SIBLING, "New Sibling Note");
                append(menu, menu::IDM_NOTES_NEW_FOLDER, "New Folder (child)");
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                append(menu, menu::IDM_NOTES_RENAME, "Rename");
                append(menu, menu::IDM_NOTES_ICON, "Change Icon...");
                append(menu, menu::IDM_NOTES_DUPLICATE, "Duplicate");
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                append(menu, menu::IDM_NOTES_MOVE_UP, "Move Up");
                append(menu, menu::IDM_NOTES_MOVE_DOWN, "Move Down");
                append(menu, menu::IDM_NOTES_MOVE_TO, "Move to...");
                append(menu, menu::IDM_NOTES_MOVE_ROOT, "Move to Top Level");
                append_common(menu, app, true);
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                append(menu, menu::IDM_NOTES_TRASH, "Move to Trash");
                append(menu, menu::IDM_NOTES_PROPERTIES, "Properties");
                SetMenuDefaultItem(menu, menu::IDM_NOTES_OPEN as u32, 0);
            }
            Some(NodeKind::Folder) => {
                append(menu, menu::IDM_NOTES_NEW, "New Note");
                append(menu, menu::IDM_NOTES_NEW_FOLDER, "New Subfolder");
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                append(menu, menu::IDM_NOTES_RENAME, "Rename");
                append(menu, menu::IDM_NOTES_ICON, "Change Icon...");
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                append(menu, menu::IDM_NOTES_MOVE_UP, "Move Up");
                append(menu, menu::IDM_NOTES_MOVE_DOWN, "Move Down");
                append(menu, menu::IDM_NOTES_MOVE_TO, "Move to...");
                append(menu, menu::IDM_NOTES_MOVE_ROOT, "Move to Top Level");
                append_common(menu, app, true);
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                append(menu, menu::IDM_NOTES_TRASH, "Move to Trash");
                append(menu, menu::IDM_NOTES_PROPERTIES, "Properties");
            }
        }
        menu
    }
}

unsafe fn append(menu: HMENU, id: u16, label: &str) {
    let text = win::wide(label);
    AppendMenuW(menu, MF_STRING, id as usize, text.as_ptr());
}

fn item_screen_pos(app: &App, id: Option<&NoteId>) -> Option<POINT> {
    let item = id.and_then(|id| item_of(app, id))?;
    let rect = row_rect(app, item)?;
    let mut point = POINT {
        x: rect.left + 12,
        y: rect.bottom,
    };
    // SAFETY: the tree window is alive.
    unsafe {
        ClientToScreen(app.tree_view, &mut point);
    }
    Some(point)
}

/// Screen rectangle of a node's full row (test hook for real mouse drags).
pub fn item_screen_rect(app: &App, id: &NoteId) -> Option<RECT> {
    let rect = row_rect(app, item_of(app, id)?)?;
    let mut top_left = POINT {
        x: rect.left,
        y: rect.top,
    };
    let mut bottom_right = POINT {
        x: rect.right,
        y: rect.bottom,
    };
    // SAFETY: the tree window is alive.
    unsafe {
        ClientToScreen(app.tree_view, &mut top_left);
        ClientToScreen(app.tree_view, &mut bottom_right);
    }
    Some(RECT {
        left: top_left.x,
        top: top_left.y,
        right: bottom_right.x,
        bottom: bottom_right.y,
    })
}

/// Focuses the tree (used when the user closes the last editor).
pub fn focus_tree(app: &App) {
    if !app.tree_view.is_null() {
        // SAFETY: our own TreeView window.
        unsafe {
            SetFocus(app.tree_view);
        }
    }
}

fn cursor_pos() -> POINT {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is a valid out parameter.
    unsafe {
        GetCursorPos(&mut point);
    }
    point
}

unsafe fn read_wide(pointer: *const u16) -> String {
    let mut length = 0usize;
    while *pointer.add(length) != 0 {
        length += 1;
        if length > 4096 {
            break;
        }
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
}
