# Application icons

`public/Lomi.icon` is the shared source, edited in Apple's Icon Composer. Its
layered artwork supplies the native macOS icon and the static Windows/Linux
exports. Do not replace the macOS asset catalog with a flattened PNG.

## Regenerate

On macOS with Xcode 26 or later selected and `pnpm install` completed:

```sh
pnpm icon:generate
```

Commit `public/Lomi.icon`, `public/app-icon.png`, and `src-tauri/icons/` together.
The command uses Icon Composer's renderer to export the default appearance,
then Tauri to generate the multi-resolution Windows ICO and Linux PNGs.
Windows/Linux builds use those checked-in files and do not require Xcode.
The PNG export also supplies the welcome screen, favicon, and README image.

The command renders six inspection previews under
`src-tauri/target/desktop-icons/previews/`: Default, Dark, ClearLight, ClearDark,
TintedLight, and TintedDark. Tinted previews use Apple's default preview color;
the installed macOS icon follows the user's chosen system tint.

## macOS packaging

`pnpm icon:macos` compiles the original `.icon` with `actool` and validates that
`Assets.car` contains the default, dark, and tintable icon stacks. macOS renders
the clear and tinted light/dark treatments from those layers. The generated
`icon.icns` is the fallback for older macOS versions.

Tauri runs this step before development, before compilation for production, and
before bundling (including standalone `tauri bundle`). The bundle contains:

- `Contents/Resources/Assets.car`, mapped by `tauri.macos.conf.json`.
- `Contents/Resources/icon.icns`, configured in the shared icon list.
- `CFBundleIconName = Lomi`, supplied by `src-tauri/Info.plist`.

The development runner creates a `.app` with the same resources. On startup,
the existing native development hook clears Tauri's static icon override so
AppKit can use the layered bundle icon.

On macOS 26 and later, select Default, Dark, Clear, or Tinted in System Settings
under Appearance and check the Dock/Finder icon in light and dark appearance.
This is an operating-system preference, independent of Lomi's editor theme.
Older macOS and Windows/Linux use the static default icon.

After replacing an installed build, quit and reopen that copy of Lomi. If the
Dock still points to an older copy, remove its shortcut and add the updated app.

References: [Apple Icon Composer](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer),
[Apple appearance workflow](https://developer.apple.com/videos/play/wwdc2025/361/),
[Tauri icons](https://v2.tauri.app/develop/icons/).
