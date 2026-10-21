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
        Kind::Program(path) => {
            // Links never start programs or scripts (a pasted link could
            // point anywhere). Offer to show the file instead.
            let question = format!(
                "This link points to a program or script:\n{path}\n\n\
                 Nodi does not run programs from links. Show it in its folder instead?"
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
        Kind::File { path, network } => {
            let confirmed = CONFIRMED.with(|set| set.borrow().contains(&path));
            if !confirmed {
                let warning = if network {
                    "\n\nThis is a network location: opening it contacts another computer."
                } else {
                    ""
                };
                let question = format!("Open this file with its default program?\n{path}{warning}");
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
    /// A document or folder on disk / on the network (asks first).
    File { path: String, network: bool },
    /// An executable or script: never run.
    Program(String),
    /// Any other scheme (`ms-settings:`, `javascript:` …).
    Refused,
}

const PROGRAM_EXTENSIONS: &[&str] = &[
    "exe",
    "com",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "psd1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "hta",
    "msi",
    "msp",
    "mst",
    "scr",
    "pif",
    "lnk",
    "cpl",
    "jar",
    "reg",
    "url",
    "application",
    "appref-ms",
    "gadget",
    "msc",
    "scf",
    "inf",
    "dll",
    "sys",
    "chm",
    "py",
    "pyw",
    "sh",
    "library-ms",
    "settingcontent-ms",
    "diagcab",
    "appx",
    "msix",
    "xbap",
];

fn classify(target: &str) -> Kind {
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        return Kind::Web;
    }
    let path = if let Some(rest) = lower.strip_prefix("file:") {
        // file:///C:/a%20b.pdf → C:\a b.pdf, file://server/share → \\server\share
        let original = &target[target.len() - rest.len()..];
        let trimmed = original.trim_start_matches('/');
        let unc = original.starts_with("//") && !original.starts_with("///");
        let decoded = percent_decode(trimmed).replace('/', "\\");
        if unc {
            format!("\\\\{decoded}")
        } else {
            decoded
        }
    } else {
        target.to_owned()
    };
    // Anything else with a scheme (but not a drive letter like C:) is refused.
    // RFC 3986 scheme: ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ).
    let has_scheme = path.find(':').is_some_and(|colon| {
        colon > 1
            && path[..1].chars().all(|ch| ch.is_ascii_alphabetic())
            && path[..colon]
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
    });
    if has_scheme {
        return Kind::Refused;
    }
    let network = path.starts_with("\\\\") || path.starts_with("//");
    let extension = std::path::Path::new(path.trim_end_matches(['\\', '/']))
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if PROGRAM_EXTENSIONS.contains(&extension.as_str()) {
        return Kind::Program(path);
    }
    Kind::File { path, network }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&text[index + 1..index + 3], 16) {
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
        Kind::Program(path) => format!("{path} (program — will not be run)"),
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
        assert_eq!(classify("https://example.com/x"), Kind::Web);
        assert_eq!(classify("mailto:a@b.c"), Kind::Web);
        assert!(matches!(
            classify(r"C:\Windows\System32\calc.exe"),
            Kind::Program(_)
        ));
        assert!(matches!(
            classify("file:///C:/Users/x/run.BAT"),
            Kind::Program(_)
        ));
        assert!(matches!(
            classify(r"C:\docs\evil.pdf.lnk"),
            Kind::Program(_)
        ));
        assert_eq!(classify("ms-settings:privacy"), Kind::Refused);
        assert_eq!(classify("javascript:alert(1)"), Kind::Refused);
        assert_eq!(
            classify("file:///C:/My%20Docs/paper.pdf"),
            Kind::File {
                path: r"C:\My Docs\paper.pdf".to_owned(),
                network: false
            }
        );
        assert!(matches!(
            classify(r"\\server\share\notes.txt"),
            Kind::File { network: true, .. }
        ));
        assert!(matches!(
            classify("file://server/share/a.docx"),
            Kind::File { network: true, .. }
        ));
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
