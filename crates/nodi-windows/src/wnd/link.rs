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
use crate::wnd::{dialogs, format, status};

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
    Some(String::from_utf16_lossy(&window[target_start..target_end]))
}

/// First `HYPERLINK "..."` target in an RTF fragment (RTF escapes undone).
pub fn hyperlink_target(rtf: &str) -> Option<String> {
    let at = rtf.find("HYPERLINK")?;
    let rest = &rtf[at + "HYPERLINK".len()..];
    let open = rest.find('"')?;
    let rest = &rest[open + 1..];
    let close = rest.find('"')?;
    Some(rtf_unescape(&rest[..close]))
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
            // SAFETY: plain key-state query.
            let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
            if !ctrl {
                return false;
            }
            let target = target_of(editor, range);
            if links::parse_math_link(&target).is_some() {
                editor::set_selection(editor, range.cp_min, range.cp_min);
            }
            follow(app, &target);
            true
        }
        WM_MOUSEMOVE => {
            let is_formula = crate::wnd::formula::formula_at(editor, range.cp_min).is_some();
            let message = if is_formula {
                "Formula: double-click or Ctrl+Click to edit".to_owned()
            } else {
                // Show where the link really goes, not its display text.
                format!(
                    "Ctrl+Click to open: {}",
                    describe(&target_of(editor, range))
                )
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
            &format!("Nodi only opens web, e-mail, file and note links. Not opened:\n{target}"),
        ),
        Kind::Reveal { path, network } => {
            // Links never start programs, scripts or unknown file types, and
            // never open files on another computer. Offer Explorer instead.
            let reason = if network {
                "a file on a network location"
            } else {
                "a program, a script or a file type Nodi does not open"
            };
            let question =
                format!("This link points to {reason}:\n{path}\n\nShow it in Explorer instead?");
            if dialogs::ask_yes_no(app.hwnd, &question) {
                shell(
                    app,
                    "open",
                    "explorer.exe",
                    Some(&format!("/select,\"{path}\"")),
                );
            }
        }
        Kind::Document(path) | Kind::Folder(path) => {
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
    }
}

thread_local! {
    /// Local file targets the user already agreed to open this session.
    static CONFIRMED: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// What following a link would do.
#[derive(Debug, PartialEq, Eq)]
enum Kind {
    /// http(s) / mailto: handed to the browser or mail client.
    Web,
    /// A local file whose type is on the document allowlist (asks first).
    Document(String),
    /// A local folder (asks first, opens in Explorer).
    Folder(String),
    /// Programs, scripts, unknown types and any file on another computer:
    /// never opened, only shown in Explorer on request.
    Reveal { path: String, network: bool },
    /// Any other scheme (`ms-settings:`, `javascript:` …) or a malformed path.
    Refused,
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
        None => target.to_owned(),
    };
    // Anything else with a scheme (but not a drive letter like C:) is refused.
    // RFC 3986 scheme: ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ).
    let scheme_like = |colon: usize| {
        colon > 1
            && path
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic())
            && path[..colon]
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
    };
    if path.find(':').is_some_and(scheme_like) {
        return Kind::Refused;
    }
    let Some(path) = normalize_path(&path) else {
        return Kind::Refused;
    };
    let network = path.starts_with("\\\\");
    if network {
        return Kind::Reveal { path, network };
    }
    if std::fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
        return Kind::Folder(path);
    }
    let extension = std::path::Path::new(&path)
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if DOCUMENT_EXTENSIONS.contains(&extension.as_str()) && !windows_says_dangerous(&extension) {
        Kind::Document(path)
    } else {
        Kind::Reveal { path, network }
    }
}

/// Normalises a path the way Win32 resolves it: forward slashes become
/// backslashes and trailing dots/spaces are dropped from every component
/// (`calc.exe.` *is* `calc.exe`). Returns `None` for paths Nodi will not touch:
/// alternate data streams (`a.txt:evil.exe`, `::$DATA`), quotes, control
/// characters, or `\\?\` device paths.
fn normalize_path(path: &str) -> Option<String> {
    let path = path.trim().replace('/', "\\");
    if path.is_empty()
        || path.starts_with("\\\\?\\")
        || path.starts_with("\\\\.\\")
        || path.chars().any(|ch| ch == '"' || ch.is_control())
    {
        return None;
    }
    // A colon is only allowed as the drive separator ("C:").
    let drive = path
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic())
        && path[1..].starts_with(':');
    if path.matches(':').count() > usize::from(drive) {
        return None;
    }
    let (prefix, rest) = if drive {
        path.split_at(2)
    } else {
        ("", path.as_str())
    };
    let components: Vec<&str> = rest
        .split('\\')
        .enumerate()
        .map(|(index, part)| {
            // Keep "." and ".." components; strip trailing dots/spaces elsewhere.
            if part == "." || part == ".." || (index == 0 && part.is_empty()) {
                part
            } else {
                part.trim_end_matches(['.', ' '])
            }
        })
        .collect();
    let joined = components.join("\\");
    let cleaned = joined.trim_end_matches('\\');
    if cleaned.is_empty() && prefix.is_empty() {
        return None;
    }
    Some(format!("{prefix}{cleaned}"))
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

/// Human-readable target for the status bar.
fn describe(target: &str) -> String {
    if let Some(id) = links::parse_note_link(target) {
        return format!("note {id}");
    }
    match classify(target) {
        Kind::Reveal { path, .. } => format!("{path} (will only be shown in Explorer)"),
        Kind::Refused => format!("{target} (not supported)"),
        _ => target.to_owned(),
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
    let target = target.replace('"', "%22");
    format!(
        "{{\\rtf1\\ansi\\uc1{{\\field{{\\*\\fldinst{{HYPERLINK \"{}\"}}}}{{\\fldrslt{{{}}}}}}}}}",
        format::rtf_escape(&target),
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
        let reveal = |target: &str| matches!(classify(target), Kind::Reveal { .. });
        assert_eq!(classify("https://example.com/x"), Kind::Web);
        assert_eq!(classify("mailto:a@b.c"), Kind::Web);
        assert!(reveal(r"C:\Windows\System32\calc.exe"));
        assert!(reveal("file:///C:/Users/x/run.BAT"));
        assert!(reveal(r"C:\docs\evil.pdf.lnk"));
        // Win32 drops trailing dots and spaces: these are still calc.exe.
        assert!(reveal(r"C:\Windows\System32\calc.exe."));
        assert!(reveal("C:/Windows/System32/calc.exe . ."));
        assert!(reveal("file:///C:/x/run.bat%20"));
        // No extension or an unknown one is never opened.
        assert!(reveal(r"C:\tools\README"));
        assert!(reveal(r"C:\x\payload.xyz"));
        // Alternate data streams, device paths and quotes are refused.
        assert_eq!(classify(r"C:\x\a.txt:evil.exe"), Kind::Refused);
        assert_eq!(classify(r"C:\x\a.txt::$DATA"), Kind::Refused);
        assert_eq!(classify(r"\\?\C:\x\a.pdf"), Kind::Refused);
        assert_eq!(classify("C:/x/a\"b.pdf"), Kind::Refused);
        assert_eq!(classify("ms-settings:privacy"), Kind::Refused);
        assert_eq!(classify("javascript:alert(1)"), Kind::Refused);
        // Documents on the allowlist open (after the confirmation prompt).
        assert_eq!(
            classify("file:///C:/My%20Docs/paper.pdf"),
            Kind::Document(r"C:\My Docs\paper.pdf".to_owned())
        );
        // Files on another computer are only shown, never opened.
        assert!(matches!(
            classify(r"\\server\share\notes.txt"),
            Kind::Reveal { network: true, .. }
        ));
        assert!(matches!(
            classify("file://server/share/a.docx"),
            Kind::Reveal { network: true, .. }
        ));
        // Multi-byte text never panics.
        assert_eq!(classify("ü:x"), Kind::Refused);
        assert!(reveal("übung"));
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
