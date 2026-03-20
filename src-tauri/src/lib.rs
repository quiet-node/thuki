/*!
 * Thuki Core Library
 *
 * Contains the shared logic and command definitions for the Thuki backend.
 */

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
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
