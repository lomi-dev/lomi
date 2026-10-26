import CoreServices
import Foundation

let allRoles = LSRolesMask(rawValue: 0xffff_ffff)

func handlers(for contentType: String) -> [String] {
  guard
    let value = LSCopyAllRoleHandlersForContentType(
      contentType as CFString,
      allRoles
    )
  else {
    return []
  }
  return ((value.takeRetainedValue() as? [String]) ?? []).sorted()
}

func defaultHandler(for contentType: String) -> String? {
  guard
    let value = LSCopyDefaultRoleHandlerForContentType(
      contentType as CFString,
      allRoles
    )
  else {
    return nil
  }
  return value.takeRetainedValue() as String
}

guard CommandLine.arguments.count == 3 else {
  fatalError("Usage: external-files-launch-services.swift APP BUNDLE_ID")
}
let appURL = URL(fileURLWithPath: CommandLine.arguments[1])
let bundleIdentifier = CommandLine.arguments[2]
let types = ["public.plain-text", "public.data"]
let before = Dictionary(
  uniqueKeysWithValues: types.map { ($0, defaultHandler(for: $0) ?? "") }
)
let status = LSRegisterURL(appURL as CFURL, true)
guard status == noErr else {
  fatalError("LSRegisterURL failed with status \(status)")
}
let records = types.map { contentType in
  let after = defaultHandler(for: contentType)
  return [
    "contentType": contentType,
    "handlers": handlers(for: contentType),
    "defaultBefore": before[contentType] ?? "",
    "defaultAfter": after ?? "",
    "defaultUnchanged": before[contentType] == (after ?? ""),
    "bundleIsHandler": handlers(for: contentType).contains(bundleIdentifier),
  ] as [String: Any]
}
let defaultsUnchanged = records.allSatisfy {
  ($0["defaultUnchanged"] as? Bool) == true
}
let appRegisteredForExpectedTypes = records.allSatisfy {
  ($0["bundleIsHandler"] as? Bool) == true
}
let report: [String: Any] = [
  "bundleIdentifier": bundleIdentifier,
  "registeredBundlePath": appURL.path,
  "types": records,
  "defaultsUnchanged": defaultsUnchanged,
  "appRegisteredForExpectedTypes": appRegisteredForExpectedTypes,
]
let data = try JSONSerialization.data(
  withJSONObject: report,
  options: [.prettyPrinted, .sortedKeys]
)
print(String(decoding: data, as: UTF8.self))
