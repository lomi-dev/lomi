import AppKit
import Foundation

guard CommandLine.arguments.count == 2,
      let pid = Int32(CommandLine.arguments[1]),
      let application = NSRunningApplication(processIdentifier: pid)
else { print("not-registered"); exit(2) }
application.activate(options: [.activateAllWindows])
RunLoop.current.run(until: Date().addingTimeInterval(0.3))
let frontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier ?? -1
print("ownPid=\(pid) frontmostPid=\(frontmost) activated=\(application.isActive)")
exit(frontmost == pid ? 0 : 3)
