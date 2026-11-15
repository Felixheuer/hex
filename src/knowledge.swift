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

// Adding the person's own appointments and reminders (`hex_calendar_add`),
// without opening Calendar or Reminders. Only with access macOS already
// granted; never asks (onboarding does). One JSON in, one JSON out.

private struct AddWhen: Codable {
    let year: Int
    let month: Int
    let day: Int
    let hour: Int?
    let minute: Int?
}

private struct AddRequest: Codable {
    let kind: String
    let title: String
    let notes: String?
    let list: String?
    let location: String?
    let start: AddWhen?
    let end: AddWhen?
    let durationMinutes: Int?
    let alertMinutes: Int?
}

private struct AddResult: Codable {
    var ok: Bool
    var error: String?
    var id: String?
    var calendar: String?
    var start: String?
    var end: String?
}

private func components(_ when: AddWhen) -> DateComponents {
    var parts = DateComponents()
    parts.calendar = Calendar.current
    parts.timeZone = TimeZone.current
    parts.year = when.year
    parts.month = when.month
    parts.day = when.day
    parts.hour = when.hour
    parts.minute = when.minute
    return parts
}

private func readable(_ date: Date, allDay: Bool) -> String {
    let format = DateFormatter()
    format.locale = Locale(identifier: "en_US_POSIX")
    format.timeZone = TimeZone.current
    format.dateFormat = allDay ? "yyyy-MM-dd" : "yyyy-MM-dd HH:mm"
    return format.string(from: date)
}

private func failed(_ text: String) -> UnsafeMutablePointer<CChar>? {
    encode(AddResult(ok: false, error: text))
}

/// The calendar (or reminder list) named `name` that takes new items, else
/// the default one.
private func target(_ store: EKEventStore, _ type: EKEntityType, _ name: String?) -> EKCalendar? {
    let fallback = type == .event
        ? store.defaultCalendarForNewEvents
        : store.defaultCalendarForNewReminders()
    guard let name, !name.trimmingCharacters(in: .whitespaces).isEmpty else { return fallback }
    let wanted = fold(name)
    let writable = store.calendars(for: type).filter(\.allowsContentModifications)
    return writable.first { fold($0.title) == wanted }
        ?? writable.first { fold($0.title).contains(wanted) }
}

@_cdecl("hex_knowledge_add")
public func hexKnowledgeAdd(_ request: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    guard let request,
          let asked = try? JSONDecoder().decode(AddRequest.self, from: Data(String(cString: request).utf8))
    else { return failed("the request couldn't be read") }
    let isEvent = asked.kind == "event"
    let type: EKEntityType = isEvent ? .event : .reminder
    guard EKEventStore.authorizationStatus(for: type) == .fullAccess else {
        return failed(isEvent
            ? "Agent-Desk has no access to Calendar (System Settings › Privacy & Security › Calendars)"
            : "Agent-Desk has no access to Reminders (System Settings › Privacy & Security › Reminders)")
    }
    let store = EKEventStore()
    guard let calendar = target(store, type, asked.list) else {
        let names = store.calendars(for: type).filter(\.allowsContentModifications).map(\.title)
        return failed("there is no \(isEvent ? "calendar" : "list") “\(asked.list ?? "")”; there are: \(names.joined(separator: ", "))")
    }
    var result = AddResult(ok: true, calendar: calendar.title)
    do {
        if isEvent {
            guard let start = asked.start, let begins = Calendar.current.date(from: components(start)) else {
                return failed("an event needs a start")
            }
            let allDay = start.hour == nil
            let ends = asked.end.flatMap { Calendar.current.date(from: components($0)) }
                ?? asked.durationMinutes.map { begins.addingTimeInterval(TimeInterval(60 * $0)) }
                ?? (allDay ? begins : begins.addingTimeInterval(3600))
            guard ends >= begins else { return failed("the end is before the start") }
            let event = EKEvent(eventStore: store)
            event.calendar = calendar
            event.title = asked.title
            event.notes = asked.notes
            event.location = asked.location
            event.isAllDay = allDay
            event.startDate = begins
            event.endDate = ends
            if let minutes = asked.alertMinutes {
                event.addAlarm(EKAlarm(relativeOffset: TimeInterval(-60 * minutes)))
            }
            try store.save(event, span: .thisEvent, commit: true)
            result.id = event.eventIdentifier
            result.start = readable(begins, allDay: allDay)
            result.end = readable(ends, allDay: allDay)
        } else {
            let reminder = EKReminder(eventStore: store)
            reminder.calendar = calendar
            reminder.title = asked.title
            reminder.notes = asked.notes
            if let due = asked.start {
                reminder.dueDateComponents = components(due)
                if due.hour != nil, let date = Calendar.current.date(from: components(due)) {
                    let offset = TimeInterval(-60 * (asked.alertMinutes ?? 0))
                    reminder.addAlarm(EKAlarm(absoluteDate: date.addingTimeInterval(offset)))
                    result.start = readable(date, allDay: false)
                } else if let date = Calendar.current.date(from: components(due)) {
                    result.start = readable(date, allDay: true)
                }
            }
            try store.save(reminder, commit: true)
            result.id = reminder.calendarItemIdentifier
        }
    } catch {
        return failed(error.localizedDescription)
    }
    return encode(result)
}
