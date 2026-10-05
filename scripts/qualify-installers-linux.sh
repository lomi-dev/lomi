#!/usr/bin/env bash
set -euo pipefail
trap 'printf "Installer check failed at line %s: %s\n" "$LINENO" "$BASH_COMMAND" >&2' ERR
mkdir -p installer-results
root="$PWD"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
deb=(src-tauri/target/release/bundle/deb/*.deb)
rpm=(src-tauri/target/release/bundle/rpm/*.rpm)
appimage=(src-tauri/target/release/bundle/appimage/*.AppImage)
test "${#deb[@]}" = 1 && test "${#rpm[@]}" = 1 && test "${#appimage[@]}" = 1
dpkg-deb --info "${deb[0]}" > installer-results/deb-metadata.txt
sudo apt-get install -y "$PWD/${deb[0]}"
dpkg-query -L lomi > installer-results/deb-files.txt
resource=$(sed -n 's@/ai-runtime/index.cjs$@@p' installer-results/deb-files.txt)
node scripts/qualify-installers-runtime.mjs /usr/bin/lomi "$resource" deb
xvfb-run -a dbus-run-session -- bash -c '
  set -euo pipefail
  /usr/bin/lomi > installer-results/deb-gui.log 2>&1 &
  app_pid=$!
  trap "kill $app_pid 2>/dev/null || true" EXIT
  for attempt in {1..90}; do
    kill -0 "$app_pid"
    if xdotool search --onlyvisible --pid "$app_pid" --name Lomi > installer-results/deb-window.txt; then exit 0; fi
    sleep 1
  done
  exit 1
'
sudo apt-get remove -y lomi
test ! -e /usr/bin/lomi
rpm -qip "${rpm[0]}" > installer-results/rpm-metadata.txt
rpm -qpR "${rpm[0]}" > installer-results/rpm-dependencies.txt
rpm --checksig "${rpm[0]}" > installer-results/rpm-integrity.txt
mkdir "$scratch/rpm"
# rpm2cpio 4.17 requires LONGARCHIVESIZE, which Tauri's RPM writer omits.
# Verify RPM digests first, then use libarchive's RPM reader.
bsdtar -xf "${rpm[0]}" -C "$scratch/rpm"
resource=$(find "$scratch/rpm" -path '*/ai-runtime/index.cjs' -printf '%h\n')
node scripts/qualify-installers-runtime.mjs "$scratch/rpm/usr/bin/lomi" "$(dirname "$resource")" rpm-extracted
docker run --rm -v "$PWD/src-tauri/target/release/bundle/rpm:/packages:ro" fedora:44 bash -euc '
  dnf install -y /packages/*.rpm
  rpm --verify lomi
  /usr/bin/lomi --mcp --version
  /usr/bin/lomi-node --version
  dnf remove -y lomi
  test ! -e /usr/bin/lomi
' > installer-results/rpm-fedora-install.log 2>&1
image="$PWD/${appimage[0]}"
chmod +x "$image"
"$image" --appimage-extract-and-run --mcp --version > installer-results/appimage-launcher.txt
grep -q '^lomi-mcp .* (control API 1.0, IPC 1)' installer-results/appimage-launcher.txt
mkdir "$scratch/appimage"
(cd "$scratch/appimage" && "$image" --appimage-extract > "$root/installer-results/appimage-files.txt")
resource=$(find "$scratch/appimage/squashfs-root" -path '*/ai-runtime/index.cjs' -printf '%h\n')
node scripts/qualify-installers-runtime.mjs "$scratch/appimage/squashfs-root/usr/bin/lomi" "$(dirname "$resource")" appimage-extracted "$scratch/rpm/usr/bin/lomi-node"
printf '%s\n' 'DEB install, visible GUI and uninstall passed; Fedora RPM install, file verification, native helper and uninstall passed; RPM and AppImage extracted runtimes and AppImage extraction launcher passed.' > installer-results/linux.txt
