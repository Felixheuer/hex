// The assistant's notifications: a Background job finished, failed, or
// asks something. Posted through UNUserNotificationCenter, which only works
// inside the bundled, signed Hex.app; clicking one opens Chats on its
// conversation (`hex://chats?id=<conversation>`).

import AppKit
import Foundation
import UserNotifications

private final class Delegate: NSObject, UNUserNotificationCenterDelegate, @unchecked Sendable {
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .list, .sound])
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        if let id = response.notification.request.content.userInfo["conversation"] as? String,
           let encoded = id.addingPercentEncoding(withAllowedCharacters: .alphanumerics.union(CharacterSet(charactersIn: "-_"))),
           let url = URL(string: "hex://chats?id=\(encoded)") {
            DispatchQueue.main.async {
                NSWorkspace.shared.open(url)
            }
        }
        completionHandler()
    }
}

private let delegate = Delegate()
private let setupLock = NSLock()
nonisolated(unsafe) private var setUp = false

/// Whether this process can post notifications itself: it runs from an app
/// bundle with a bundle id.
@_cdecl("hex_assistant_notifications_available")
public func hexAssistantNotificationsAvailable() -> Bool {
    Bundle.main.bundleIdentifier != nil && Bundle.main.bundlePath.hasSuffix(".app")
}

/// Posts a notification (asking for permission the first time); returns at
/// once. `conversation` may be empty.
@_cdecl("hex_assistant_notify")
public func hexAssistantNotify(
    _ title: UnsafePointer<CChar>,
    _ body: UnsafePointer<CChar>,
    _ conversation: UnsafePointer<CChar>
) -> Bool {
    guard hexAssistantNotificationsAvailable() else { return false }
    let title = String(cString: title)
    let body = String(cString: body)
    let conversation = String(cString: conversation)
    let center = UNUserNotificationCenter.current()
    setupLock.lock()
    if !setUp {
        center.delegate = delegate
        setUp = true
    }
    setupLock.unlock()
    center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
        guard granted else { return }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        if !conversation.isEmpty {
            content.userInfo = ["conversation": conversation]
            content.threadIdentifier = conversation
        }
        let request = UNNotificationRequest(
            identifier: UUID().uuidString, content: content, trigger: nil)
        center.add(request)
    }
    return true
}
