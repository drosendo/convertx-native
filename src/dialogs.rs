//! Native file dialogs. Android has no such thing, so the pickers are no-ops
//! there and the queue is filled by the file manager's "share" instead.

use std::path::PathBuf;

#[cfg(not(target_os = "android"))]
pub fn pick_files() -> Option<Vec<PathBuf>> {
    rfd::FileDialog::new()
        .set_title("Choose files")
        .pick_files()
}

#[cfg(target_os = "android")]
pub fn pick_files() -> Option<Vec<PathBuf>> {
    None
}

#[cfg(not(target_os = "android"))]
pub fn pick_folder() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Choose output folder")
        .pick_folder()
}

#[cfg(target_os = "android")]
pub fn pick_folder() -> Option<PathBuf> {
    None
}

/// Used by "Locate…" when a tool lives somewhere unusual.
#[cfg(not(target_os = "android"))]
pub fn pick_file(title: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title(title).pick_file()
}

#[cfg(target_os = "android")]
pub fn pick_file(_title: &str) -> Option<PathBuf> {
    None
}
