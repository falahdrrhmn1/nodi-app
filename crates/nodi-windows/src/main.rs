#![windows_subsystem = "windows"]

mod app;
mod explorer;
mod prefs;
mod testhook;
mod themes;
mod win;
mod wnd;

fn main() {
    std::process::exit(wnd::run());
}
