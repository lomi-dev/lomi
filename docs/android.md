# Local Android phones

Android runs inside a Lomi tab or docked panel. It uses a local Android
Emulator process with its own persistent virtual phone data, without opening an
emulator window or requiring Android Studio.

Native qualification currently covers the recorded Apple M3/macOS ARM64 host.
Android setup and Start are disabled on Windows, Linux and Intel macOS until
their native lifecycle, input and graphics tests are completed. This restriction
applies to Android, not to the rest of Lomi. See the exact host, tool
versions, results and remaining limitations in [the architecture guide](android-architecture.md).
The modern profile and zoom extension was tested with Android 17 (API 37.2)
Google Play and Pixel 10 Pro XL at 1344 × 2992, using a scaled preview. The
report distinguishes the thirty-minute transport trial from a two-minute
regression after the viewport retention fix. Other catalog images are offered
according to SDK compatibility; they have not all been individually tested.

## Prepare a phone

1. Choose **+ → New android symulator** and then **Set up Android**.
2. In **Settings → Android**, follow the setup card. It shows the next step:
   install tools, download an Android version, then create a phone. Any required
   system preparation appears above it.
3. Choose **Install Android tools**, review the provider terms and approve
   the required licenses. Lomi downloads its own SDK tools and private
   Java runtime into its local application data. It does not change the
   system PATH, JAVA_HOME, shell profiles or another application's SDK.
4. Open **Android versions**. Recent stable versions appear first, with filters
   for Android version and included apps. Choose **Google Play** for Play Store
   and Google services, **Google APIs** for services without the store, or
   **AOSP** for a minimal open-source system. Images marked **16 KB** require
   compatible native libraries in your apps. Download only the image you want
   after reviewing its terms; preview and Canary builds are excluded.
5. Create a device using an available phone profile and installed image. Review
   the bundled Lomi input method, which provides Unicode and composition
   inside this virtual phone. Profiles come from the installed official tools,
   including recent Pixel models. Profiles needing a newer Android version are
   disabled with an explanation. The name initially follows your chosen phone;
   you can change it. Hardware options remain under **Advanced settings**.
   Host GPU and cold boot are the tested configuration.
6. Choose **Open** next to your phone to return to the workspace. The phone’s
   **…** menu contains configuration, default phone selection, cold boot,
   wipe and delete. During setup, **Open in new tab** creates an additional
   view if the original panel has been closed.

Once set up, **Your phones** is the main view. Running phones have a **Stop**
button. **Android versions** holds downloads and installed systems; **Advanced**
holds SDK details, storage, repair and recovery. Optional hardware settings stay
collapsed in the device form. Deleting or wiping a phone still requires typing
its exact name.

Image downloads show their current stage, progress, transferred bytes and a
**Cancel** button beside the Android version in **Android versions**. The active
download stays visible when filters change or Settings reopens. Verification
and installation show an indeterminate bar until the native operation finishes.
Closing Settings keeps an explicitly started installation running. Closing
Lomi safely settles or cancels the installation before exiting.

## Use the panel

Without a default phone, a new panel lists your phones with their Android
version and current state. Select a row to open that phone. A configured default
still opens automatically; a missing saved phone lets you select a replacement.

Startup shows three stages: **Prepare phone**, **Start Android** and **Connect
screen**. Stages follow the actual startup and screen connection; they do not
estimate a percentage. **Cancel start** stays available while startup is pending,
including before Android reports its first status. The panel waits for a
confirmed stop before offering Start again. Multiple views share one pending
start, and switching tabs after cancellation does not restart the phone.

Click and drag on the screen to touch, scroll to swipe, or Alt-drag for a
mirrored two-finger gesture. Type while the phone has focus; Tab leaves the
phone. Application shortcuts and ordinary form fields keep their normal roles.
Use **Paste** to transfer the host clipboard intentionally. Typing does not
use the clipboard, and there is no background clipboard synchronization.

The vertical toolbar on the right provides Android navigation, screen power,
rotation, APK installation and screenshot capture. Close stays pinned at the
top; the other controls scroll in short panels. Use **Zoom in**, **Zoom out**
or **Fit to panel** in the actions menu to resize the preview. Pinch or
Ctrl/Cmd-scroll over the phone to zoom around the pointer. Middle-drag or
Shift-scroll pans an enlarged preview; ordinary scrolling swipes in Android.
Each view keeps its own zoom and position across docking and workspace changes
during the application session.

The phone retains its profile's full screen resolution and density. Continuous
previews are scaled at the emulator to at most 1280 pixels on either edge and
921,600 pixels total. **100%** maps a preview pixel to a physical display pixel;
enlarging a large phone's preview does not reveal extra detail. **Actual size
(1:1)** is offered only when the phone fits the preview budget; other phones
offer **Preview size (100%)**. **Save screenshot…** exports full phone resolution.

The actions menu includes rotation, Start/Stop/Restart, zoom, **Install APK…**,
**Save screenshot…** and **Device details**. The latter exposes the
current ADB serial and managed ADB path for your own terminal work. Selecting
an APK installs it; Lomi does not build a project or execute copied text.

Two views of the same device share its apps, data, process and image stream.
Create another device for an independent phone. Docking, resizing and switching
workspaces preserve the running process. Hidden or minimized views stop image
transfer; Android itself still occupies RAM until **Stop**. Closing the final
view stops the device and preserves its data. Restored tabs start their phones
only when visited.

Modern Google Play images can use substantially more host memory than AOSP.
The tested Android 17 emulator and its helper used roughly 7–8.3 GiB, including
graphics and emulator allocations; the configured guest RAM is only part of
that total. Stop unused phones to release their resources.

## Manage and recover

Settings manages names, the default device, hardware, images and maintenance.
Hardware changes apply on the next start after Stop. A different system image
requires a new device. An image referenced by any device cannot be updated or
removed in place, including while the device is stopped.

**Wipe data…** and **Delete device…** require the exact device name. Cache
cleanup preserves phone data. Use **Export diagnostics** for bounded local
logs. Failed starts and disconnected images offer an explicit retry, reconnect
or route back to setup; a missing restored device remains repairable in its tab.

The Android backend never kills a shared ADB server or uses a regular ADB client
that might replace an incompatible server. A conflicting server or unverified
device identity produces an error to resolve before retrying. A second
Lomi process cannot manage the same Android directory concurrently.

Profiles describe virtual screen hardware, not every physical component or a
manufacturer's exclusive software. Foldable/resizable profiles remain excluded
until their changing displays and input mapping are implemented and tested.

Quick Boot, host cameras/microphone, audio forwarding, physical phones, iOS,
cloud devices and Android AI/MCP control are outside the qualified v1 setup.
