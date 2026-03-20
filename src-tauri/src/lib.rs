/*!
 * Thuki Core Library
 *
 * Contains the shared logic and command definitions for the Thuki backend.
 */

pub mod commands;

/// Starts the Tauri application.
///
/// This function initializes the Tauri workspace, sets up the communication handlers,
/// and runs the main event loop. It serves as the entry point for both desktop and
/// mobile platforms.
///
/// # Panics
///
/// Panics if there is an error during the initialization or while running the application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            use tauri::Manager;
            app.manage(reqwest::Client::new());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::ask_ollama])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
