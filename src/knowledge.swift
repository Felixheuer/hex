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
    let addresses: [String]
    let me: Bool
}

/// Words that find the person's own card ("me" in Contacts).
private let meWords = "me ich mich mein meine my myself selbst"

private struct BridgeReminder: Codable {
    let id: String
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
        CNContactPostalAddressesKey,
    ] as [CNKeyDescriptor]
    let request = CNContactFetchRequest(keysToFetch: keys)
    request.unifyResults = true
    let store = CNContactStore()
    let meId = (try? store.unifiedMeContactWithKeys(toFetch: [CNContactIdentifierKey as CNKeyDescriptor]))?.identifier
    let formatter = CNPostalAddressFormatter()
    let cap = Int(max(limit, 1))
    var found: [BridgeContact] = []
    do {
        try store.enumerateContacts(with: request) { contact, stop in
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
            let addresses = contact.postalAddresses.map { labeled in
                formatter.string(from: labeled.value)
                    .split(whereSeparator: \.isNewline)
                    .joined(separator: ", ")
            }
            let isMe = contact.identifier == meId
            let haystack = fold(
                ([name, contact.nickname, contact.organizationName, contact.jobTitle, isMe ? meWords : ""]
                    + emails + phones.map(\.number) + digits + addresses).joined(separator: " ")
            )
            guard tokens.allSatisfy({ haystack.contains($0) }) else { return }
            found.append(BridgeContact(
                name: name,
                nickname: contact.nickname,
                organization: contact.organizationName,
                jobTitle: contact.jobTitle,
                phones: phones,
                emails: emails,
                addresses: addresses,
                me: isMe
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
                id: reminder.calendarItemIdentifier,
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
    let action: String
    let id: String?
    let kind: String
    let title: String?
    let notes: String?
    let list: String?
    let location: String?
    let start: AddWhen?
    let end: AddWhen?
    let durationMinutes: Int?
    let alertMinutes: Int?
    let completed: Bool?
}

private struct AddResult: Codable {
    var ok: Bool
    var error: String?
    var id: String?
    var calendar: String?
    var start: String?
    var end: String?
    var title: String?
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

private func alarmed(_ item: EKCalendarItem, _ alarm: EKAlarm?) {
    for old in item.alarms ?? [] { item.removeAlarm(old) }
    if let alarm { item.addAlarm(alarm) }
}

/// The reminder's alarm: `minutes` before its due time (a time is needed).
private func reminderAlarm(_ due: AddWhen?, _ minutes: Int?) -> EKAlarm? {
    guard let due, due.hour != nil, let date = Calendar.current.date(from: components(due)) else { return nil }
    return EKAlarm(absoluteDate: date.addingTimeInterval(TimeInterval(-60 * (minutes ?? 0))))
}

private func saveEvent(_ store: EKEventStore, _ event: EKEvent, _ asked: AddRequest, isNew: Bool) throws -> AddResult? {
    if let title = asked.title { event.title = title }
    if let notes = asked.notes { event.notes = notes }
    if let location = asked.location { event.location = location }
    if let list = asked.list {
        guard let calendar = target(store, .event, list) else { return nil }
        event.calendar = calendar
    }
    let oldLength = isNew ? nil : event.endDate.timeIntervalSince(event.startDate)
    if let start = asked.start, let begins = Calendar.current.date(from: components(start)) {
        let allDay = start.hour == nil
        event.isAllDay = allDay
        event.startDate = begins
        event.endDate = asked.end.flatMap { Calendar.current.date(from: components($0)) }
            ?? asked.durationMinutes.map { begins.addingTimeInterval(TimeInterval(60 * $0)) }
            ?? oldLength.map { begins.addingTimeInterval($0) }
            ?? (allDay ? begins : begins.addingTimeInterval(3600))
    } else if let end = asked.end.flatMap({ Calendar.current.date(from: components($0)) }) {
        event.endDate = end
    } else if let minutes = asked.durationMinutes {
        event.endDate = event.startDate.addingTimeInterval(TimeInterval(60 * minutes))
    }
    guard event.endDate >= event.startDate else {
        throw NSError(domain: "hex", code: 1, userInfo: [NSLocalizedDescriptionKey: "the end is before the start"])
    }
    if let minutes = asked.alertMinutes {
        alarmed(event, EKAlarm(relativeOffset: TimeInterval(-60 * minutes)))
    }
    try store.save(event, span: .thisEvent, commit: true)
    return AddResult(
        ok: true, id: event.eventIdentifier, calendar: event.calendar?.title,
        start: readable(event.startDate, allDay: event.isAllDay),
        end: readable(event.endDate, allDay: event.isAllDay), title: event.title
    )
}

private func saveReminder(_ store: EKEventStore, _ reminder: EKReminder, _ asked: AddRequest) throws -> AddResult? {
    if let title = asked.title { reminder.title = title }
    if let notes = asked.notes { reminder.notes = notes }
    if let list = asked.list {
        guard let calendar = target(store, .reminder, list) else { return nil }
        reminder.calendar = calendar
    }
    if let due = asked.start {
        reminder.dueDateComponents = components(due)
        alarmed(reminder, reminderAlarm(due, asked.alertMinutes))
    }
    if let completed = asked.completed { reminder.isCompleted = completed }
    try store.save(reminder, commit: true)
    let due = reminder.dueDateComponents.flatMap { Calendar.current.date(from: $0) }
    return AddResult(
        ok: true, id: reminder.calendarItemIdentifier, calendar: reminder.calendar?.title,
        start: due.map { readable($0, allDay: reminder.dueDateComponents?.hour == nil) },
        title: reminder.title
    )
}

/// Adds, changes, completes, or deletes one event or reminder.
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
    let noList = {
        let names = store.calendars(for: type).filter(\.allowsContentModifications).map(\.title)
        return failed("there is no \(isEvent ? "calendar" : "list") “\(asked.list ?? "")”; there are: \(names.joined(separator: ", "))")
    }
    do {
        if asked.action == "add" {
            guard let calendar = target(store, type, asked.list) else { return noList() }
            let item: AddResult?
            if isEvent {
                let event = EKEvent(eventStore: store)
                event.calendar = calendar
                item = try saveEvent(store, event, asked, isNew: true)
            } else {
                let reminder = EKReminder(eventStore: store)
                reminder.calendar = calendar
                item = try saveReminder(store, reminder, asked)
            }
            return item.map { encode($0) } ?? noList()
        }
        guard let id = asked.id else { return failed("changing or deleting needs the id from hex_lookup") }
        let found: EKCalendarItem? = isEvent
            ? store.event(withIdentifier: id) ?? store.calendarItem(withIdentifier: id)
            : store.calendarItem(withIdentifier: id)
        guard let found, found.calendar?.allowsContentModifications ?? false else {
            return failed("there is no \(isEvent ? "event" : "reminder") with the id \(id) that can be changed")
        }
        if asked.action == "delete" {
            let title = found.title
            if let event = found as? EKEvent {
                try store.remove(event, span: .thisEvent, commit: true)
            } else if let reminder = found as? EKReminder {
                try store.remove(reminder, commit: true)
            }
            return encode(AddResult(ok: true, id: id, calendar: found.calendar?.title, title: title))
        }
        let item: AddResult?
        if let event = found as? EKEvent {
            item = try saveEvent(store, event, asked, isNew: false)
        } else if let reminder = found as? EKReminder {
            item = try saveReminder(store, reminder, asked)
        } else {
            return failed("the id \(id) is neither an event nor a reminder")
        }
        return item.map { encode($0) } ?? noList()
    } catch {
        return failed(error.localizedDescription)
    }
}
