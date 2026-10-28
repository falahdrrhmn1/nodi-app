//! Hyperlinks: web addresses, local files and links between notes
//! (`nodi://note/<id>`, so renaming a note never breaks them).
//!
//! Links are RTF `HYPERLINK` fields with a friendly display text. Ctrl+Click
//! follows a link (plain clicks keep editing the text, as in Word).

use std::ptr;

use nodi_core::links;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::NMHDR;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, SW_SHOWNORMAL};

use crate::app::App;
use crate::win;
use crate::wnd::editor::{self, CharRange};
use crate::wnd::form::{Field, Form};
use crate::wnd::{clipboard, dialogs, format, status};

pub const EN_LINK: u32 = 0x070B;
const EM_AUTOURLDETECT: u32 = 0x0400 + 91;
const AURL_ENABLEURL: usize = 1;
const AURL_ENABLEEMAILADDR: usize = 2;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_MOUSEMOVE: u32 = 0x0200;

/// `ENLINK` (richedit.h, `pack(4)`): the `WPARAM` lands on offset 28.
#[repr(C, packed(4))]
pub struct EnLink {
    pub header: NMHDR,
    pub msg: u32,
    pub wparam: usize,
    pub lparam: isize,
    pub range: CharRange,
}

const _: () = assert!(std::mem::size_of::<EnLink>() == 52);

/// Typed web addresses and e-mail addresses become links automatically.
pub fn enable_auto_detect(editor: HWND) {
    // SAFETY: plain message to our Rich Edit.
    unsafe {
        SendMessageW(
            editor,
            EM_AUTOURLDETECT,
            AURL_ENABLEURL | AURL_ENABLEEMAILADDR,
            0,
        );
    }
}

/// The address behind the link covering `range`: the `HYPERLINK` field
/// instruction for a friendly link, or the text itself for a detected URL.
pub fn target_of(editor: HWND, range: CharRange) -> String {
    // The hidden field instruction sits right before the link text in the
    // raw text; reading it there avoids touching the selection.
    if let Some(target) = raw_target(&editor::raw_text(editor), range.cp_min.max(0) as usize) {
        return target;
    }
    let (from, to) = editor::selection(editor);
    editor::set_selection(editor, range.cp_min, range.cp_max);
    let rtf = editor::selection_rtf(editor)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    editor::set_selection(editor, from, to);
    hyperlink_target(&rtf).unwrap_or_else(|| {
        editor::text_range(editor, range.cp_min, range.cp_max)
            .trim()
            .to_owned()
    })
}

/// Target of a friendly link whose (visible) text starts at `start`: the raw
/// text reads `HYPERLINK "target" [switches]` immediately before it.
fn raw_target(raw: &[u16], start: usize) -> Option<String> {
    let start = start.min(raw.len());
    if start == 0 || raw[start - 1] != u16::from(b'"') {
        return None;
    }
    let window_start = start.saturating_sub(4096);
    let window = &raw[window_start..start];
    let needle: Vec<u16> = "HYPERLINK \"".encode_utf16().collect();
    let found = window
        .windows(needle.len())
        .rposition(|candidate| candidate == needle.as_slice())?;
    let instruction = &window[found..];
    // The instruction never spans paragraphs; otherwise this is an older,
    // unrelated link.
    if instruction.contains(&0x000D) {
        return None;
    }
    let target_start = found + needle.len();
    let target_end = window[target_start..]
        .iter()
        .position(|unit| *unit == u16::from(b'"'))
        .map(|offset| target_start + offset)?;
    Some(normalize_target(String::from_utf16_lossy(
        &window[target_start..target_end],
    )))
}

/// Trims a target and removes duplicated separators from `X:\` paths. Rich
/// Edit's own field bookkeeping sometimes leaves `C:\\...` in the instruction
/// when a link carries a trailing space (see `link_rtf`); Win32 treats the two
/// forms the same, but links must read back cleanly.
fn normalize_target(target: String) -> String {
    let target = target.trim_end();
    // Path links are stored as file URLs (Rich Edit drops a field whose target
    // is a bare `C:\...`); report the path the user sees again.
    if let Some(path) = links::file_url_path(target) {
        return path;
    }
    if target.as_bytes().get(1) == Some(&b':') {
        target.replace("\\\\", "\\")
    } else {
        target.to_owned()
    }
}

/// First `HYPERLINK "..."` target in an RTF fragment (RTF escapes undone).
pub fn hyperlink_target(rtf: &str) -> Option<String> {
    let at = rtf.find("HYPERLINK")?;
    let rest = &rtf[at + "HYPERLINK".len()..];
    let open = rest.find('"')?;
    let rest = &rest[open + 1..];
    let close = rest.find('"')?;
    Some(normalize_target(rtf_unescape(&rest[..close])))
}

fn rtf_unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('{') => out.push('{'),
                Some('}') => out.push('}'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// `EN_LINK` from an editor. Ctrl+Click opens; hovering shows the target.
pub fn on_link(app: &mut App, link: &EnLink) -> bool {
    let editor = link.header.hwndFrom;
    let range = link.range;
    match link.msg {
        WM_LBUTTONUP => {
            let target = target_of(editor, range);
            // A code block's Copy link works with a plain click (it is not a
            // navigation link); editing the text is never the intent there.
            if links::is_copy_link(&target) {
                if let Some(code) = format::code_block_text(editor, range.cp_min) {
                    if clipboard::set_text(editor, &code) {
                        status::set_message(app, "Code copied");
                    }
                }
                return true;
            }
            // SAFETY: plain key-state query.
            let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
            if !ctrl {
                return false;
            }
            if links::parse_math_link(&target).is_some() {
                editor::set_selection(editor, range.cp_min, range.cp_min);
            }
            follow(app, &target);
            true
        }
        WM_MOUSEMOVE => {
            let target = target_of(editor, range);
            let is_formula = crate::wnd::formula::formula_at(editor, range.cp_min).is_some();
            let message = if links::is_copy_link(&target) {
                "Click to copy this code block".to_owned()
            } else if is_formula {
                "Formula: double-click or Ctrl+Click to edit".to_owned()
            } else {
                // Show where the link really goes, not its display text.
                format!("Ctrl+Click to open: {}", describe(&target))
            };
            status::set_message(app, &message);
            false
        }
        _ => false,
    }
}

/// Opens a link target: a formula (editor), another note, a file or a web page.
pub fn follow(app: &mut App, target: &str) {
    if links::parse_math_link(target).is_some() {
        crate::wnd::formula::insert_or_edit(app);
        return;
    }
    if let Some(id) = links::parse_note_link(target) {
        let usable = app.tree.get(&id).is_some_and(|node| !node.trashed);
        if usable {
            app.selected = Some(id.clone());
            app.open_note(id);
        } else {
            dialogs::error(
                app.hwnd,
                "The linked note no longer exists (or is in the Trash).",
            );
        }
        return;
    }
    let target = target.trim();
    if target.is_empty() {
        return;
    }
    match classify(target) {
        Kind::Web => shell(app, "open", target, None),
        Kind::Refused => dialogs::error(
            app.hwnd,
            &format!(
                "Nodi only opens web, e-mail, note links and files on this computer \
                 (C:\\..., D:\\...). Not opened:\n{target}"
            ),
        ),
        Kind::Network(path) => network_refused(app, &path),
        Kind::Local(path) => {
            // The file system is only touched here, after a deliberate
            // Ctrl+Click, and never on a network drive (that would contact the
            // other computer and could hand it the Windows login hash).
            if drive_is_remote(&path) {
                network_refused(app, &path);
                return;
            }
            let is_dir = std::fs::metadata(&path).is_ok_and(|meta| meta.is_dir());
            match local_action(&path, is_dir) {
                LocalAction::Open => {
                    let confirmed = CONFIRMED.with(|set| set.borrow().contains(&path));
                    if !confirmed {
                        let question = format!("Open this with its default program?\n{path}");
                        if !dialogs::ask_yes_no(app.hwnd, &question) {
                            return;
                        }
                        CONFIRMED.with(|set| set.borrow_mut().insert(path.clone()));
                    }
                    shell(app, "open", &path, None);
                }
                LocalAction::Reveal => {
                    // Programs, scripts, shortcuts and unknown types are never
                    // run from a link; Explorer can show where they are.
                    let question = format!(
                        "This link points to a program, a script or a file type Nodi does \
                         not open:\n{path}\n\nShow it in Explorer instead?"
                    );
                    if dialogs::ask_yes_no(app.hwnd, &question) {
                        shell(
                            app,
                            "open",
                            "explorer.exe",
                            Some(&format!("/select,\"{path}\"")),
                        );
                    }
                }
            }
        }
    }
}

fn network_refused(app: &App, path: &str) {
    dialogs::info(
        app.hwnd,
        &format!(
            "This link points to a network location:\n{path}\n\n\
             Nodi does not open network locations from links, because that \
             contacts another computer. Open it yourself in Explorer if you trust it."
        ),
    );
}

thread_local! {
    /// Local targets the user already agreed to open this session.
    static CONFIRMED: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// What a link target is. Deciding this never touches the file system, so
/// hovering a link has no side effects.
#[derive(Debug, PartialEq, Eq)]
enum Kind {
    /// http(s) / mailto: handed to the browser or mail client.
    Web,
    /// A fully qualified path on a drive letter (`X:\...`), normalised.
    Local(String),
    /// `\\server\share\...`: never opened or touched.
    Network(String),
    /// Other schemes, relative or device paths, streams, reserved names.
    Refused,
}

/// What to do with a local path once it was clicked.
#[derive(Debug, PartialEq, Eq)]
enum LocalAction {
    /// A folder, or a document type on the allowlist (asks first).
    Open,
    /// Anything else: only shown in Explorer on request.
    Reveal,
}

/// File types a link may open with their default program. Everything else
/// (programs, scripts, shortcuts, unknown types) is only shown in Explorer:
/// an allowlist cannot be bypassed by an extension nobody thought of.
const DOCUMENT_EXTENSIONS: &[&str] = &[
    "pdf", "txt", "md", "markdown", "rtf", "log", "csv", "tsv", "json", "xml", "yaml", "yml",
    "toml", "ini", "tex", "bib", "doc", "docx", "odt", "xls", "xlsx", "ods", "ppt", "pptx", "odp",
    "epub", "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico", "heic", "mp3", "wav",
    "flac", "ogg", "m4a", "mp4", "mkv", "avi", "mov", "webm", "zip", "7z", "rar", "tar", "gz",
    "ipynb", "rs", "c", "h", "cpp", "hpp", "cs", "java", "go", "kt", "sql", "css", "drawio",
    "nodi",
];

fn classify(target: &str) -> Kind {
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        return Kind::Web;
    }
    let path = match lower.strip_prefix("file:") {
        // file:///C:/a%20b.pdf → C:\a b.pdf, file://server/share → \\server\share
        // (`to_ascii_lowercase` keeps byte offsets, and "file:" is ASCII.)
        Some(rest) => {
            let original = &target[target.len() - rest.len()..];
            let trimmed = original.trim_start_matches('/');
            let unc = original.starts_with("//") && !original.starts_with("///");
            let decoded = percent_decode(trimmed).replace('/', "\\");
            if unc {
                format!("\\\\{decoded}")
            } else {
                decoded
            }
        }
        None => target.trim().replace('/', "\\"),
    };
    // `\\server\share` is a network location. `\\?\`, `\\.\` and `\??\`
    // (device / NT object paths, which can also reach the network) and any
    // other path starting with a backslash are refused outright.
    if let Some(rest) = path.strip_prefix("\\\\") {
        let device = rest.starts_with('?') || rest.starts_with('.');
        return if device || rest.is_empty() {
            Kind::Refused
        } else {
            Kind::Network(path)
        };
    }
    match normalize_path(&path) {
        Some(path) => Kind::Local(path),
        None => Kind::Refused,
    }
}

/// Accepts only fully qualified drive paths (`X:\...`) and normalises them the
/// way Win32 resolves them: trailing dots/spaces are dropped from every
/// component (`calc.exe.` *is* `calc.exe`). Returns `None` for relative paths,
/// alternate data streams (`a.txt:evil.exe`, `::$DATA`), quotes, control
/// characters and reserved device names (`CON`, `NUL`, `COM1`, …).
fn normalize_path(path: &str) -> Option<String> {
    let path = path.trim();
    let bytes = path.as_bytes();
    let fully_qualified =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    if !fully_qualified
        || path.matches(':').count() != 1
        || path
            .chars()
            .any(|ch| ch == '"' || ch.is_control() || ch == '*' || ch == '?')
    {
        return None;
    }
    // `bytes[0..3]` are ASCII, so slicing at 3 is on a char boundary.
    let (drive, rest) = path.split_at(3);
    let mut components = Vec::new();
    for part in rest.split('\\') {
        if part.is_empty() {
            continue;
        }
        let part = if part == "." || part == ".." {
            part
        } else {
            part.trim_end_matches(['.', ' '])
        };
        if part.is_empty() || is_reserved_device(part) {
            return None;
        }
        components.push(part);
    }
    Some(format!(
        "{}{}",
        drive.to_ascii_uppercase(),
        components.join("\\")
    ))
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM0-9`, `LPT0-9`, `CONIN$`, `CONOUT$` — with
/// or without an extension, these name devices, not files.
fn is_reserved_device(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.as_bytes()[3].is_ascii_digit())
}

/// Folder → open; allowlisted document type (that Windows does not flag) →
/// open; anything else → reveal only.
fn local_action(path: &str, is_dir: bool) -> LocalAction {
    if is_dir {
        return LocalAction::Open;
    }
    let extension = std::path::Path::new(path)
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if DOCUMENT_EXTENSIONS.contains(&extension.as_str()) && !windows_says_dangerous(&extension) {
        LocalAction::Open
    } else {
        LocalAction::Reveal
    }
}

/// True unless the path's drive is a local disk, removable drive, CD or RAM
/// disk (mapped network drives and unknown roots count as remote).
pub(crate) fn drive_is_remote(path: &str) -> bool {
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_CDROM: u32 = 5;
    const DRIVE_RAMDISK: u32 = 6;
    let Some(root) = path.get(..3) else {
        return true;
    };
    let root = win::wide(root);
    // SAFETY: NUL-terminated root such as "C:\"; the call only queries.
    let kind = unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root.as_ptr()) };
    !matches!(
        kind,
        DRIVE_REMOVABLE | DRIVE_FIXED | DRIVE_CDROM | DRIVE_RAMDISK
    )
}

/// Windows' own list of dangerous file types (`AssocIsDangerous`), as a
/// second opinion on top of the allowlist.
fn windows_says_dangerous(extension: &str) -> bool {
    if extension.is_empty() {
        return true;
    }
    let dotted = win::wide(&format!(".{extension}"));
    // SAFETY: NUL-terminated string; the call only reads the registry.
    unsafe { windows_sys::Win32::UI::Shell::AssocIsDangerous(dotted.as_ptr()) != 0 }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            // Work on bytes: `index + 3` may fall inside a multi-byte char.
            let value = std::str::from_utf8(&bytes[index + 1..index + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(value) = value {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Human-readable target for the status bar. Pure: hovering a link never
/// touches the file system or the network.
fn describe(target: &str) -> String {
    if let Some(id) = links::parse_note_link(target) {
        return format!("note {id}");
    }
    match classify(target) {
        Kind::Web => target.to_owned(),
        Kind::Local(path) => path,
        Kind::Network(path) => format!("{path} (network location — not opened)"),
        Kind::Refused => format!("{target} (not supported)"),
    }
}

fn shell(app: &App, verb: &str, file: &str, parameters: Option<&str>) {
    let verb = win::wide(verb);
    let file_wide = win::wide(file);
    let parameters = parameters.map(win::wide);
    // SAFETY: all strings are NUL terminated and outlive the call.
    let result = unsafe {
        ShellExecuteW(
            app.hwnd,
            verb.as_ptr(),
            file_wide.as_ptr(),
            parameters.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (result as isize) <= 32 {
        dialogs::error(app.hwnd, &format!("Windows could not open:\n{file}"));
    }
}

/// RTF for a friendly hyperlink showing `text` and pointing at `target`.
pub fn link_rtf(text: &str, target: &str) -> String {
    let target = target.trim();
    // Rich Edit silently drops a field whose target is a bare local path, so
    // store those as file URLs; the display text stays exactly as given.
    let stored = links::path_file_url(target).unwrap_or_else(|| target.to_owned());
    let stored = stored.replace('"', "%22");
    format!(
        "{{\\rtf1\\ansi\\uc1{{\\field{{\\*\\fldinst{{HYPERLINK \"{}\"}}}}{{\\fldrslt{{{}}}}}}}}}",
        format::rtf_escape(&stored),
        format::rtf_escape(text)
    )
}

/// Insert > Link... (Ctrl+K): create a link, or edit the selected one.
pub fn insert_or_edit(app: &mut App) {
    let Some(editor) = app.active_editor() else {
        return;
    };
    let (from, to) = editor::selection(editor);
    let selected_rtf = editor::selection_rtf(editor)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let existing = if from != to {
        hyperlink_target(&selected_rtf)
    } else {
        None
    };
    let text = if from != to {
        editor::text_range(editor, from, to)
            .trim_end_matches('\r')
            .to_owned()
    } else {
        String::new()
    };

    let notes = app.visible_titles();
    let form = Form::new("Link")
        .note("Address: a web page (https://…), a file path, or pick a note.")
        .field(Field::new("Text:", text))
        .field(Field::new("Address:", existing.unwrap_or_default()))
        .extra(
            "Browse File...",
            Box::new(|owner, values| {
                if let Some(path) = dialogs::open_any_file(owner) {
                    if values[0].trim().is_empty() {
                        values[0] = std::path::Path::new(&path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.clone());
                    }
                    values[1] = path;
                }
            }),
        )
        .extra(
            "Browse Folder...",
            Box::new(|owner, values| {
                if let Some(path) = dialogs::open_folder(owner) {
                    if values[0].trim().is_empty() {
                        values[0] = path.clone();
                    }
                    values[1] = path;
                }
            }),
        )
        .extra(
            "Link to Note...",
            Box::new(move |owner, values| {
                if let Some(id) = dialogs::find_note(owner, &notes) {
                    if values[0].trim().is_empty() {
                        if let Some((_, title)) = notes.iter().find(|(note, _)| *note == id) {
                            values[0] = title.clone();
                        }
                    }
                    values[1] = links::format_note_link(&id);
                }
            }),
        );
    let Some(values) = form.show(app.hwnd) else {
        return;
    };
    let target = values[1].trim().to_owned();
    let mut text = values[0].trim().to_owned();
    if target.is_empty() {
        return;
    }
    if text.is_empty() {
        text = target.clone();
    }
    editor::set_selection(editor, from, to);
    if editor::insert_rtf(editor, link_rtf(&text, &target).as_bytes()) {
        app.mark_dirty(editor);
    } else {
        dialogs::error(app.hwnd, "Rich Edit refused the link.");
    }
}

/// Turns the local path just before the caret into a link (typed text, a
/// closing quote or Enter). Returns false when there is no path to convert.
pub fn auto_link_before_caret(editor: HWND) -> bool {
    if editor::in_table(editor) {
        return false;
    }
    let (from, to) = editor::selection(editor);
    if from != to {
        return false;
    }
    let start = editor::paragraph_start(editor, from);
    let text = editor::text_range(editor, start, from);
    // A quoted path may contain spaces: wait for the closing quote instead of
    // ending the path at the first space.
    if text.matches('"').count() % 2 == 1 {
        return false;
    }
    let Some((offset, path)) = links::local_path_suffix(&text) else {
        return false;
    };
    let begin = start + text[..offset].encode_utf16().count() as i32;
    // Do not wrap a link that is already there (its display text may itself
    // be a path).
    if raw_target(&editor::raw_text(editor), begin.max(0) as usize).is_some() {
        return false;
    }
    let path = path.to_owned();
    let before = editor::text_length(editor);
    editor::set_selection(editor, begin, from);
    if !editor::insert_rtf(editor, link_rtf(&path, &path).as_bytes()) {
        editor::set_selection(editor, from, from);
        return false;
    }
    // Rich Edit counts a field's hidden instruction text, so the caret is
    // placed from the document-length change, never from the visible text.
    let caret = begin + (editor::text_length(editor) - before) + (from - begin);
    editor::set_selection(editor, caret, caret);
    true
}

/// A pasted plain text that is exactly one local path becomes a link.
pub fn paste_path_link(editor: HWND, text: &str) -> bool {
    let Some(path) = links::whole_local_path(text) else {
        return false;
    };
    let path = path.to_owned();
    let (at, _) = editor::selection(editor);
    let before = editor::text_length(editor);
    if !editor::insert_rtf(editor, link_rtf(&path, &path).as_bytes()) {
        return false;
    }
    let caret = at + (editor::text_length(editor) - before);
    editor::set_selection(editor, caret, caret);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_hyperlink_target() {
        let rtf = r#"{\rtf1{\field{\*\fldinst{HYPERLINK "nodi://note/abc"}}{\fldrslt{Gradient}}}}"#;
        assert_eq!(hyperlink_target(rtf).as_deref(), Some("nodi://note/abc"));
        assert_eq!(
            hyperlink_target(r#"HYPERLINK "C:\\Users\\x.pdf""#).as_deref(),
            Some(r"C:\Users\x.pdf")
        );
        assert!(hyperlink_target("{\\rtf1 plain}").is_none());
    }

    #[test]
    fn classify_never_runs_programs() {
        let local = |target: &str| match classify(target) {
            Kind::Local(path) => path,
            other => panic!("{target}: expected a local path, got {other:?}"),
        };
        let action = |target: &str| local_action(&local(target), false);
        assert_eq!(classify("https://example.com/x"), Kind::Web);
        assert_eq!(classify("mailto:a@b.c"), Kind::Web);
        // Programs, scripts and shortcuts are only revealed, never opened.
        assert_eq!(action(r"C:\Windows\System32\calc.exe"), LocalAction::Reveal);
        assert_eq!(action("file:///C:/Users/x/run.BAT"), LocalAction::Reveal);
        assert_eq!(action(r"C:\docs\evil.pdf.lnk"), LocalAction::Reveal);
        // Win32 drops trailing dots and spaces: these are still calc.exe.
        assert_eq!(
            local(r"C:\Windows\System32\calc.exe."),
            r"C:\Windows\System32\calc.exe"
        );
        assert_eq!(
            action("C:/Windows/System32/calc.exe . ."),
            LocalAction::Reveal
        );
        assert_eq!(action("file:///C:/x/run.bat%20"), LocalAction::Reveal);
        // No extension or an unknown one is never opened.
        assert_eq!(action(r"C:\tools\README"), LocalAction::Reveal);
        assert_eq!(action(r"C:\x\payload.xyz"), LocalAction::Reveal);
        // Documents on the allowlist (and folders) open after a prompt.
        assert_eq!(
            local("file:///C:/My%20Docs/paper.pdf"),
            r"C:\My Docs\paper.pdf"
        );
        assert_eq!(action("file:///C:/My%20Docs/paper.pdf"), LocalAction::Open);
        assert_eq!(local_action(r"C:\Users", true), LocalAction::Open);
        // Streams, device and NT object paths, relative paths, reserved
        // names and quotes are refused.
        assert_eq!(classify(r"C:\x\a.txt:evil.exe"), Kind::Refused);
        assert_eq!(classify(r"C:\x\a.txt::$DATA"), Kind::Refused);
        assert_eq!(classify(r"\\?\C:\x\a.pdf"), Kind::Refused);
        assert_eq!(classify(r"\\.\pipe\x"), Kind::Refused);
        assert_eq!(classify(r"\??\UNC\server\share\a.pdf"), Kind::Refused);
        assert_eq!(classify(r"\server\share\a.pdf"), Kind::Refused);
        assert_eq!(classify(r"..\a.pdf"), Kind::Refused);
        assert_eq!(classify("a.pdf"), Kind::Refused);
        assert_eq!(classify(r"C:x.pdf"), Kind::Refused);
        assert_eq!(classify(r"C:\x\CON"), Kind::Refused);
        assert_eq!(classify(r"C:\x\nul.txt"), Kind::Refused);
        assert_eq!(classify(r"C:\x\COM1.pdf"), Kind::Refused);
        assert_eq!(classify("C:/x/a\"b.pdf"), Kind::Refused);
        assert_eq!(classify(r"C:\x\a*.pdf"), Kind::Refused);
        assert_eq!(classify("ms-settings:privacy"), Kind::Refused);
        assert_eq!(classify("javascript:alert(1)"), Kind::Refused);
        // Network locations are recognised but never opened or touched.
        assert!(matches!(
            classify(r"\\server\share\notes.txt"),
            Kind::Network(_)
        ));
        assert!(matches!(
            classify("file://server/share/a.docx"),
            Kind::Network(_)
        ));
        assert!(describe(r"\\server\share\a.pdf").contains("not opened"));
        // Multi-byte text never panics.
        assert_eq!(classify("ü:x"), Kind::Refused);
        assert_eq!(classify("übung"), Kind::Refused);
        assert_eq!(local(r"C:\übung\Ä.pdf"), r"C:\übung\Ä.pdf");
        assert_eq!(percent_decode("%€x%4"), "%€x%4");
    }

    #[test]
    fn raw_target_reads_the_hidden_instruction() {
        let raw: Vec<u16> = "intro\rsee HYPERLINK \"https://x.y/a\"Docs"
            .encode_utf16()
            .collect();
        let start = raw.len() - 4;
        assert_eq!(raw_target(&raw, start).as_deref(), Some("https://x.y/a"));
        // A link on an earlier line is not borrowed by plain text.
        let raw: Vec<u16> = "HYPERLINK \"https://x.y\"A\r\"quoted\""
            .encode_utf16()
            .collect();
        assert!(raw_target(&raw, raw.len()).is_none());
    }

    #[test]
    fn link_rtf_round_trips_target() {
        let rtf = link_rtf("Docs {x}", r"C:\a b\c.txt");
        assert_eq!(hyperlink_target(&rtf).as_deref(), Some(r"C:\a b\c.txt"));
        assert!(rtf.contains("Docs \\{x\\}"));
    }
}
