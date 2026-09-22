import AppKit
import Foundation

// Preserve every clipboard representation without logging or retaining its contents.
let board = NSPasteboard.general
let mode = CommandLine.arguments[1]
let directory = URL(fileURLWithPath: CommandLine.arguments[2])
let backup = directory.appendingPathComponent("clipboard-backup.plist")
let marker = NSPasteboard.PasteboardType("dev.lomi.clipboard-smoke")
switch mode {
case "backup":
    let items = (board.pasteboardItems ?? []).map { item in
        Dictionary(uniqueKeysWithValues: item.types.compactMap { type in
            item.data(forType: type).map { (type.rawValue, $0) }
        })
    }
    let data = try PropertyListSerialization.data(fromPropertyList: items, format: .binary, options: 0)
    try data.write(to: backup, options: .atomic)
    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: backup.path)
case "restore":
    defer { try? FileManager.default.removeItem(at: backup) }
    if board.string(forType: marker) == directory.path {
        let data = try Data(contentsOf: backup)
        let items = try PropertyListSerialization.propertyList(from: data, format: nil) as! [[String: Data]]
        board.clearContents()
        board.writeObjects(items.map { values in
            let item = NSPasteboardItem()
            for (type, bytes) in values { item.setData(bytes, forType: NSPasteboard.PasteboardType(type)) }
            return item
        })
    }
case "image", "image-only", "text":
    board.clearContents()
    board.setString(directory.path, forType: marker)
    if mode != "text" {
        let png = try Data(contentsOf: directory.appendingPathComponent("fixture.png"))
        board.setData(png, forType: .png)
        if let tiff = NSBitmapImageRep(data: png)?.tiffRepresentation { board.setData(tiff, forType: .tiff) }
        if mode == "image" { board.setString("https://example.test/fixture.png", forType: .string) }
    } else {
        board.setString("zażółć 🦀\r\nsecond line", forType: .string)
    }
default:
    fatalError("Unknown clipboard fixture mode")
}
