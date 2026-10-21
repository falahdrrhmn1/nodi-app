#![windows_subsystem = "windows"]

mod app;
mod win;
mod wnd;

fn main() {
    std::process::exit(wnd::run());
}
