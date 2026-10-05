import Cocoa

let pid = Int32(CommandLine.arguments[1])!
let deadline = Date().addingTimeInterval(90)
while Date() < deadline {
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    if let window = windows.first(where: {
        ($0[kCGWindowOwnerPID as String] as? Int32) == pid &&
        ($0[kCGWindowLayer as String] as? Int) == 0 &&
        (($0[kCGWindowBounds as String] as? [String: Any])?["Width"] as? Double ?? 0) >= 800
    }) {
        print(window)
        exit(0)
    }
    Thread.sleep(forTimeInterval: 1)
}
fputs("Installed application did not show its main window.\n", stderr)
exit(1)
