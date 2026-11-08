# SolidCraft for Android

The desktop app's engine and interface in an Android app, for arm64 phones and tablets (a
foldable unfolded is the best fit: the interface is the desktop one). The app itself is
`apps/solidcraft-android`; this folder is the APK around it.

## Getting the APK

Every push to the `android` branch builds one on GitHub Actions (`.github/workflows/android.yml`)
and publishes it as a pre-release named "SolidCraft Android N". On the phone, open the release,
download `SolidCraft-android-N.apk`, open it, and allow installing apps from the browser or file
manager when Android asks. A newer build installs over an older one and keeps its settings.
The APK is large (about 180 MB): the whole CAD engine, its fonts and the render library.

To build it yourself: `packaging/android/build.sh` (its first lines say what it needs).

## Using it

- **Touch**, as in touch CAD apps:
  - one finger dragged from empty space turns the view (pans it on a flat page); on a face it
    pulls it out or in (Extrude or cut, with Preferences › Tools › Direct interaction, on by
    default here), on sketch geometry it drags it; a tap selects, a tap on empty space clears
    the selection;
  - two fingers pan, and pinch to zoom, at the same time;
  - three fingers swiped left undo, swiped right redo;
  - two fingers tapped together are a right click (context menus);
  - a finger held still before it moves draws a selection box, and held longer opens Select
    Other;
  - while a sketch tool runs, one finger draws (a drag draws a rectangle or circle), so two
    fingers navigate there.
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
