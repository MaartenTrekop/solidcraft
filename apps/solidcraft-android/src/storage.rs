//! "All files access" (`MANAGE_EXTERNAL_STORAGE`): with it the app reads and writes the phone's
//! shared storage through ordinary file paths, as the desktop app does, so opening, saving,
//! recent files, linked designs and the lock files beside open designs all work unchanged.
//! Android grants it only on its own settings page, which [`ask_for_all_files_access`] opens.

use jni::objects::{JObject, JValue};
use jni::{JavaVM, jni_sig, jni_str};
use winit::platform::android::activity::AndroidApp;

/// The app's package, as in packaging/android/app/build.gradle.kts (`applicationId`).
pub const PACKAGE: &str = "io.github.maartentrekop.solidcraft";

/// Whether the app may manage all files: `None` when Android could not be asked.
pub fn has_all_files_access(app: &AndroidApp) -> Option<bool> {
    let vm = java_vm(app)?;
    let asked = vm.attach_current_thread(|env| -> jni::errors::Result<bool> {
        env.call_static_method(jni_str!("android/os/Environment"), jni_str!("isExternalStorageManager"), jni_sig!("()Z"), &[])?.z()
    });
    match asked {
        Ok(allowed) => Some(allowed),
        Err(e) => {
            log::warn!("storage: could not ask Android about all files access: {e}");
            None
        }
    }
}

/// Open Android's page where the user allows all files access for this app (or, where that page
/// is missing, the list of every app that asks for it).
pub fn ask_for_all_files_access(app: &AndroidApp) -> Result<(), String> {
    let vm = java_vm(app).ok_or("Android's Java VM is not available")?;
    let activity = app.activity_as_ptr() as jni::sys::jobject;
    if activity.is_null() {
        return Err("the activity is gone".into());
    }
    let for_app = open_settings(&vm, activity, "android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION", true);
    match for_app {
        Ok(()) => Ok(()),
        Err(e) => {
            log::warn!("storage: the app's all files access page did not open ({e}); opening the list instead");
            open_settings(&vm, activity, "android.settings.MANAGE_ALL_FILES_ACCESS_PERMISSION", false).map_err(|e| e.to_string())
        }
    }
}

/// `activity.startActivity(new Intent(action[, Uri.parse("package:…")]))`.
#[allow(unsafe_code)]
fn open_settings(vm: &JavaVM, activity: jni::sys::jobject, action: &str, for_this_app: bool) -> jni::errors::Result<()> {
    vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        // SAFETY: a global reference to the running activity that android-activity holds for the
        // life of the activity; it is only borrowed here, never deleted.
        let activity = unsafe { JObject::from_raw(env, activity) };
        let action = env.new_string(action)?;
        let intent = if for_this_app {
            let text = env.new_string(format!("package:{PACKAGE}"))?;
            let uri = env
                .call_static_method(jni_str!("android/net/Uri"), jni_str!("parse"), jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(&text)])?
                .l()?;
            env.new_object(jni_str!("android/content/Intent"), jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"), &[JValue::Object(&action), JValue::Object(&uri)])?
        } else {
            env.new_object(jni_str!("android/content/Intent"), jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&action)])?
        };
        env.call_method(&activity, jni_str!("startActivity"), jni_sig!("(Landroid/content/Intent;)V"), &[JValue::Object(&intent)])?;
        Ok(())
    })
}

#[allow(unsafe_code)]
fn java_vm(app: &AndroidApp) -> Option<JavaVM> {
    let raw = app.vm_as_ptr() as *mut jni::sys::JavaVM;
    if raw.is_null() {
        return None;
    }
    // SAFETY: android-activity hands out the process's Java VM, which lives as long as the process.
    Some(unsafe { JavaVM::from_raw(raw) })
}
