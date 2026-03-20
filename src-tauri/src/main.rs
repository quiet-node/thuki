// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// The main entry point for the desktop application.
///
/// This function calls into the common core library to start the application.
fn main() {
    thuki_agent_lib::run()
}
