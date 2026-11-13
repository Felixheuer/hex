// The assistant's notifications: a Background job finished, failed, or
// asks something. Posted through UNUserNotificationCenter, which only works
// inside the bundled, signed Agent-Desk.app. A click is handed back to Rust in
// process (`hex_assistant_notifications_on_click`), which projects the job's
// result on the overlay's stage; the notification's own "Open Agent-Desk"
// action asks for the window. No `agent-desk://` link: the app isn't activated
// and the window doesn't open by itself.

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
           !id.isEmpty,
           let clicked = clickHandler {
            let window = response.actionIdentifier == openWindowAction
            DispatchQueue.main.async {
                id.withCString { clicked($0, window) }
            }
        }
        completionHandler()
    }
}

/// What Rust does with a click: the conversation, and whether it was "Open Agent-Desk".
public typealias ClickHandler = @convention(c) (UnsafePointer<CChar>, Bool) -> Void
nonisolated(unsafe) private var clickHandler: ClickHandler?
private let category = "hex-assistant-job"
private let openWindowAction = "open-window"

/// Takes clicks in process from now on, and adds the "Open Agent-Desk" action
/// (titled `openTitle`, in the interface's language) to every job notification.
@_cdecl("hex_assistant_notifications_on_click")
public func hexAssistantNotificationsOnClick(
    _ clicked: ClickHandler,
    _ openTitle: UnsafePointer<CChar>
) {
    clickHandler = clicked
    let title = String(cString: openTitle)
    let center = UNUserNotificationCenter.current()
    setupLock.lock()
    center.delegate = delegate
    setUp = true
    setupLock.unlock()
    let open = UNNotificationAction(identifier: openWindowAction, title: title, options: [.foreground])
    center.setNotificationCategories([
        UNNotificationCategory(identifier: category, actions: [open], intentIdentifiers: [], options: [])
    ])
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
            content.categoryIdentifier = category
        }
        let request = UNNotificationRequest(
            identifier: UUID().uuidString, content: content, trigger: nil)
        center.add(request)
    }
    return true
}
