//! SolidCraft on Android: the desktop app's window, engine and UI in a NativeActivity.
//!
//! Android loads this library and calls `android_main` (src/android.rs) on a thread of its own.
//! The host differs from the desktop one where a phone or tablet does:
//!
//! - files: an Open and Save dialog of its own over the phone's storage (src/files.rs), with
//!   "All files access" asked for once (src/storage.rs), since the system chooser only hands out
//!   content URIs the app's commands cannot read;
//! - touch: one finger is the left mouse button, two or more fingers a camera gesture
//!   ([`touch`]); the Back button is Escape;
//! - lifetime: settings and unsaved designs are kept whenever Android asks the app to save,
//!   because it may end the process at any time once the app is in the background.
//!
//! Building and installing: `packaging/android/README.md`.

pub mod touch;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
mod files;
#[cfg(target_os = "android")]
mod storage;
