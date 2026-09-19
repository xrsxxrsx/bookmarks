// Prevent a console window from appearing alongside the application on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    bookmarks_app_lib::run()
}
