#!/usr/bin/env bash
set -euo pipefail
mkdir -p installer-results
scratch=$(mktemp -d)
trap 'hdiutil detach "$scratch/mount" >/dev/null 2>&1 || true; rm -rf "$scratch"' EXIT
dmg=(src-tauri/target/release/bundle/dmg/*.dmg)
test "${#dmg[@]}" = 1
hdiutil verify "${dmg[0]}"
hdiutil attach "${dmg[0]}" -nobrowse -mountpoint "$scratch/mount"
ditto "$scratch/mount/Lomi.app" "$scratch/Lomi.app"
hdiutil detach "$scratch/mount"
codesign --verify --deep --strict --verbose=2 "$scratch/Lomi.app"
codesign -d --entitlements :- "$scratch/Lomi.app/Contents/MacOS/lomi-node" > installer-results/macos-entitlements.plist
python3 -c 'import plistlib; assert plistlib.load(open("installer-results/macos-entitlements.plist", "rb"))["com.apple.security.cs.allow-jit"] is True'
test -s "$scratch/Lomi.app/Contents/Resources/Assets.car"
node scripts/qualify-installers-runtime.mjs "$scratch/Lomi.app/Contents/MacOS/lomi" "$scratch/Lomi.app/Contents/Resources" dmg
"$scratch/Lomi.app/Contents/MacOS/lomi" > installer-results/macos-gui.log 2>&1 &
app_pid=$!
trap 'kill "$app_pid" 2>/dev/null || true; rm -rf "$scratch"' EXIT
swift scripts/qualify-installers-window.swift "$app_pid" > installer-results/macos-window.txt
kill "$app_pid"
wait "$app_pid" || true
rm -rf "$scratch/Lomi.app"
test ! -e "$scratch/Lomi.app"
printf '%s\n' 'DMG integrity, copy-install, ad-hoc signature, runtime, visible GUI and removal passed.' > installer-results/macos.txt
