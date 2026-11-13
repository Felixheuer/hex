import Contacts
import EventKit
import Foundation

// The assistant's lookups in Contacts and Reminders (`hex_lookup`). Both
// read only when macOS already granted access, and never ask: a read without
// access would show the system prompt from a tool call. Onboarding asks
// (`hex_knowledge_*_request`, at the end).

private struct BridgePhone: Codable {
    let label: String
    let number: String
}

private struct BridgeContact: Codable {
    let name: String
    let nickname: String
    let organization: String
    let jobTitle: String
    let phones: [BridgePhone]
    let emails: [String]
}

private struct BridgeReminder: Codable {
    let title: String
    let list: String
    let dueMs: Double?
    let priority: Int
}

private func encode<T: Encodable>(_ value: T) -> UnsafeMutablePointer<CChar>? {
    guard let data = try? JSONEncoder().encode(value) else { return nil }
    return strdup(String(decoding: data, as: UTF8.self))
}

private func fold(_ text: String) -> String {
    text.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: nil).lowercased()
}

/// The query's words; a number's leading zeros and plus dropped, so
/// "0162 1234" meets "+49 162 1234".
private func words(_ query: UnsafePointer<CChar>?) -> [String] {
    guard let query else { return [] }
    return fold(String(cString: query))
        .split(whereSeparator: { $0.isWhitespace })
        .map { word -> String in
            let digits = word.filter { $0 != "+" }
            if !digits.isEmpty && digits.allSatisfy(\.isNumber) {
                return String(digits.drop(while: { $0 == "0" }))
            }
            return String(word)
        }
        .filter { !$0.isEmpty }
}

@_cdecl("hex_knowledge_free")
public func hexKnowledgeFree(_ string: UnsafeMutablePointer<CChar>?) {
    free(string)
}

@_cdecl("hex_knowledge_contacts_status")
public func hexKnowledgeContactsStatus() -> Int32 {
    Int32(CNContactStore.authorizationStatus(for: .contacts).rawValue)
}

@_cdecl("hex_knowledge_contacts")
public func hexKnowledgeContacts(_ query: UnsafePointer<CChar>?, _ limit: Int32) -> UnsafeMutablePointer<CChar>? {
    let status = CNContactStore.authorizationStatus(for: .contacts).rawValue
    guard status == 3 || status == 4 else { return nil }
    let tokens = words(query)
    let keys = [
        CNContactGivenNameKey, CNContactFamilyNameKey, CNContactNicknameKey,
        CNContactOrganizationNameKey, CNContactJobTitleKey,
        CNContactPhoneNumbersKey, CNContactEmailAddressesKey,
    ] as [CNKeyDescriptor]
    let request = CNContactFetchRequest(keysToFetch: keys)
    request.unifyResults = true
    let cap = Int(max(limit, 1))
    var found: [BridgeContact] = []
    do {
        try CNContactStore().enumerateContacts(with: request) { contact, stop in
            let phones = contact.phoneNumbers.map { labeled in
                BridgePhone(
                    label: labeled.label.map { CNLabeledValue<CNPhoneNumber>.localizedString(forLabel: $0) } ?? "",
                    number: labeled.value.stringValue
                )
            }
            let emails = contact.emailAddresses.map { String($0.value) }
            let name = [contact.givenName, contact.familyName]
                .filter { !$0.isEmpty }
                .joined(separator: " ")
            let digits = phones.map { $0.number.filter(\.isNumber) }
            let haystack = fold(
                ([name, contact.nickname, contact.organizationName, contact.jobTitle]
                    + emails + phones.map(\.number) + digits).joined(separator: " ")
            )
            guard tokens.allSatisfy({ haystack.contains($0) }) else { return }
            found.append(BridgeContact(
                name: name,
                nickname: contact.nickname,
                organization: contact.organizationName,
                jobTitle: contact.jobTitle,
                phones: phones,
                emails: emails
            ))
            if found.count >= cap { stop.pointee = true }
        }
    } catch {
        return nil
    }
    return encode(found)
}

@_cdecl("hex_knowledge_reminders_status")
public func hexKnowledgeRemindersStatus() -> Int32 {
    Int32(EKEventStore.authorizationStatus(for: .reminder).rawValue)
}

private final class Fetched: @unchecked Sendable {
    var reminders: [EKReminder] = []
}

@_cdecl("hex_knowledge_reminders")
public func hexKnowledgeReminders(_ query: UnsafePointer<CChar>?, _ limit: Int32) -> UnsafeMutablePointer<CChar>? {
    guard EKEventStore.authorizationStatus(for: .reminder) == .fullAccess else { return nil }
    let tokens = words(query)
    let store = EKEventStore()
    let predicate = store.predicateForIncompleteReminders(
        withDueDateStarting: nil, ending: nil, calendars: nil
    )
    let fetched = Fetched()
    let done = DispatchSemaphore(value: 0)
    store.fetchReminders(matching: predicate) { reminders in
        fetched.reminders = reminders ?? []
        done.signal()
    }
    guard done.wait(timeout: .now() + 3) == .success else { return nil }
    let cap = Int(max(limit, 1))
    let found = fetched.reminders
        .filter { reminder in
            let haystack = fold((reminder.title ?? "") + " " + (reminder.calendar?.title ?? ""))
            return tokens.allSatisfy { haystack.contains($0) }
        }
        .prefix(cap)
        .map { reminder in
            BridgeReminder(
                title: reminder.title ?? "",
                list: reminder.calendar?.title ?? "",
                dueMs: reminder.dueDateComponents
                    .flatMap { Calendar.current.date(from: $0) }
                    .map { $0.timeIntervalSince1970 * 1000 },
                priority: reminder.priority
            )
        }
    return encode(Array(found))
}

// Asking for access: only from onboarding and its repair guide, never from
// a tool call. Each shows macOS's prompt when it was never answered and
// blocks the calling (background) thread until the person answers.
// Returns the framework's status code, a grant as authorized (3) even
// while the status still reads "not determined" in this process, and a
// refusal as denied (2).

private final class Granted: @unchecked Sendable {
    var value = false
}

private let reminderStore = EKEventStore()

@_cdecl("hex_knowledge_contacts_request")
public func hexKnowledgeContactsRequest() -> Int32 {
    let done = DispatchSemaphore(value: 0)
    let granted = Granted()
    CNContactStore().requestAccess(for: .contacts) { allowed, _ in
        granted.value = allowed
        done.signal()
    }
    done.wait()
    if granted.value { return 3 }
    let status = CNContactStore.authorizationStatus(for: .contacts).rawValue
    return status == 0 ? 2 : Int32(status)
}

@_cdecl("hex_knowledge_reminders_request")
public func hexKnowledgeRemindersRequest() -> Int32 {
    let done = DispatchSemaphore(value: 0)
    let granted = Granted()
    reminderStore.requestFullAccessToReminders { allowed, _ in
        granted.value = allowed
        done.signal()
    }
    done.wait()
    if granted.value { return 3 }
    let status = EKEventStore.authorizationStatus(for: .reminder).rawValue
    return status == 0 ? 2 : Int32(status)
}
