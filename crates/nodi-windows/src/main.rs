#![windows_subsystem = "windows"]

mod app;
mod testhook;
mod win;
mod wnd;

fn main() {
    std::process::exit(wnd::run());
}
