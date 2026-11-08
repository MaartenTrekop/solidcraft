# SolidCraft for Android

The desktop app's engine and interface in an Android app, for arm64 phones and tablets (a
foldable unfolded is the best fit: the interface is the desktop one). The app itself is
`apps/solidcraft-android`; this folder is the APK around it.

## Getting the APK

Every push to the `android` branch builds one on GitHub Actions (`.github/workflows/android.yml`)
and publishes it as a pre-release named "SolidCraft Android N". On the phone, open the release,
download `SolidCraft-android-N.apk`, open it, and allow installing apps from the browser or file
manager when Android asks. A newer build installs over an older one and keeps its settings.

To build it yourself: `packaging/android/build.sh` (its first lines say what it needs).

## Using it

- **Touch**: a tap is a click; one finger drags (geometry, a selection box, sliders, panels);
  two fingers turn the view when they move together or zoom when they pinch, whichever they
  start with; three fingers pan. Two fingers tapped together are a right click (context menus).
  Holding a finger still is a held mouse button (Select Other).
- **Back** is Escape: it cancels a tool or closes a dialog.
- **Files**: Open and Save show SolidCraft's own file dialog. The first time, tap **Allow** and
  turn on "Allow access to manage all files" for SolidCraft: the app then opens and saves
  anywhere in the phone's storage (it suggests `Documents/SolidCraft`). Without it, designs are
  kept in the app's own folder.
- **Keyboard**: the on-screen keyboard opens for text fields; a Bluetooth keyboard and mouse
  work too (the mouse's clicks act as taps).
- **Size**: Preferences › Appearance › Interface scale (0.9 on a first start).
- Unsaved designs are kept for recovery whenever the app goes to the background, since Android
  may close it there; the next start offers them back.

## Limits

- arm64 only (every recent phone).
- No clipboard exchange with other apps, no drag and drop, no 3D mouse, no agent chat agents
  that run programs, no update check (new builds come from the workflow).
- The signing key (`sideload.keystore`, password `android`) is in the repository so that builds
  install over each other; it is only fit for installing your own builds.

## Logs

`adb logcat -s SolidCraft` shows the app's log, including the graphics adapter it chose and any
panic.
