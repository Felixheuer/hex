//! The runner's public facade: what the Action shortcut, the overlay, the
//! Assistant pane, and `hex assistant` call. Each goes to the one runner.

use super::*;

/// Queues `request` and returns its conversation's id at once (a new id when
/// `request.conversation` is `None`).
///
/// - A request for a conversation that is running **steers** it: its current
///   turn stops, and it goes on in the same conversation (same backend
///   session, windows, and browser) told "The person interrupted you while
///   you were doing X … Their new instruction: …". The stopped turn ends
///   without a final event; the steered run then reports `Started` and ends
///   with the conversation's one `Answer`/`Failed`/`Cancelled`.
/// - A new request (`conversation: None`) that names one running task by
///   its title or apps ("bei der Mail-Aufgabe …") steers that task.
/// - Otherwise a Here request stops any other Here task; Background and
///   Chat tasks run beside each other.
pub fn submit(request: Request) -> ConversationId {
    runner().submit(request)
}

/// Passes `text` to the running job `to` names (its short id from the
/// stream's digest, or words of its title) as the person's instruction;
/// whether one was found (`hex_tell_job`).
pub fn tell_job(to: &str, text: &str) -> bool {
    runner().tell_job(to, text)
}

/// A hold of the Action shortcut just became real: starts the backend for
/// the next `lane` request while the person is still speaking. Returns at
/// once; for the Here lane it reads the window in front itself (a few
/// milliseconds, off the caller's thread). The next matching `submit` (same
/// lane, same window, same follow-up conversation) finds the process and its
/// MCP servers up. Cheap to call again; does nothing for backends that can't
/// start ahead (only Claude Code can) or while the assistant is off.
pub fn prewarm(lane: Lane) {
    std::thread::Builder::new()
        .name("assistant-prewarm".into())
        .spawn(move || {
            let window = match lane {
                Lane::Here => crate::assistant::computer::quick_context().window,
                Lane::Background => None,
            };
            runner().prewarm(lane, window);
        })
        .ok();
}

/// The words of the Action shortcut's hold so far (the live transcript, as
/// it streams): what they will likely need starts now, before the release
/// (the reserved quick conversation's browser when they ask for the web).
/// Cheap and quiet to call on every change; does nothing outside a hold.
pub fn speculate(partial: &str) {
    if partial.trim().is_empty()
        || crate::assistant::interaction::listening()
            != crate::assistant::interaction::Listening::Hold
    {
        return;
    }
    runner().speculate(partial);
}

/// `prewarm` for a window the caller already knows (`hex assistant ask`).
pub fn prewarm_for(lane: Lane, window: Option<TaskWindow>) {
    runner().prewarm(lane, window);
}

/// Stops one task: its agent process, its desktop actions, and its speech.
/// It ends with `Event::Cancelled`.
pub fn cancel(conversation: &str) {
    runner().cancel(conversation);
}

/// Stops every task.
pub fn cancel_all() {
    runner().cancel_all();
}

/// The tasks running or queued now, oldest first.
pub fn tasks() -> Vec<TaskStatus> {
    runner().tasks()
}

/// The questions tasks wait on: the one the next hold answers, and the
/// others behind it (`Event::QuestionsChanged` tells when it changes).
pub fn inbox() -> Inbox {
    runner().inbox()
}

/// The person is holding the Action shortcut or dictating: questions and
/// finished jobs wait to be read aloud until they stop (or 30 s pass).
pub fn set_listening(on: bool) {
    voice::set_listening(on, store::now_ms());
}

/// Hex goes quiet at once: the line being read stops and the rest of that
/// answer is dropped; questions and summaries waiting their turn stay
/// (`interaction::hush`).
pub fn hush() {
    let runner = runner();
    runner.voice.stop();
    (runner.hooks.stop_speaking)();
}

/// Whether questions and finished jobs are read aloud (the default), or
/// only shown, for a surface that plays a soft chime instead.
#[allow(dead_code, reason = "for the overlay's chime setting")]
pub fn set_spoken_announcements(on: bool) {
    voice::set_spoken_announcements(on);
}

/// Whether an answer is still being read aloud or is about to be: its
/// sentences wait their turn, or more of it is still streaming. Together
/// with `speech::speaking()` (what the engine plays or makes now), it tells
/// conversation mode the reading isn't over between two sentences.
pub fn reading_aloud() -> bool {
    runner().voice.reading()
}

/// Whether questions and finished jobs are read aloud, for the overlay
/// to chime when they aren't.
pub fn spoken_announcements() -> bool {
    voice::spoken_announcements()
}

/// Puts `conversation`'s question behind the others (the overlay's
/// Later; saying "später" does the same), and brings up the next.
pub fn defer_question(conversation: &str) {
    runner().defer(conversation);
}

/// Whether any task is running or queued.
pub fn busy() -> bool {
    !runner().state().tasks.is_empty()
}

/// The screen frame's state (Agent mode is gone: the frame follows the
/// work, `frame_shown`).
pub fn agent_mode() -> bool {
    frame_shown()
}

/// Whether the screen frame shows: any task is working (a Here question
/// thinking or acting, or Background work), not just waiting for the
/// person's answer (`session::frame_visible`).
pub fn frame_shown() -> bool {
    runner().state().frame_on
}

/// Kept for callers of the old Agent mode switch: turning it off stops every
/// task; turning it on does nothing (Background work turns the frame on).
pub fn set_agent_mode(on: bool) {
    if !on {
        stop_all();
    }
}

/// Stops every task (the Stop all shortcut, the stack's and Chats' button,
/// the menu bar), and ends backends started ahead.
pub fn stop_all() {
    cancel_all();
    provider::cool_down();
}

/// Whether Background work is running, waiting, paused, or queued: the
/// screen frame shows while this is true.
pub fn background_busy() -> bool {
    runner().state().background_on
}

/// The person takes `conversation` over: its desktop actions wait (through
/// the guard), and its window comes forward. `Event::Paused`.
pub fn take_over(conversation: &str) {
    runner().take_over(conversation);
}

/// Gives `conversation` back after `take_over`; it looks again before acting.
/// `Event::Resumed`.
pub fn hand_back(conversation: &str) {
    runner().hand_back(conversation);
}

/// Continues a conversation Hex quit or crashed in the middle of
/// (`Outcome::Interrupted`), telling the agent to check what was already
/// done before going on. A Here conversation goes on in the Background lane:
/// its window may be gone. `None` when it wasn't interrupted.
pub fn resume_interrupted(conversation: &str) -> Option<ConversationId> {
    runner().resume_interrupted(conversation)
}

/// The plan of a running or finished job (its `plan.md` checklist), if it
/// made one.
pub fn plan(conversation: &str) -> Vec<PlanStep> {
    runner().plan(conversation)
}

/// The question a task is waiting on (`TaskState::NeedsInput`), if any.
pub fn question(conversation: &str) -> Option<String> {
    runner().question(conversation)
}

/// Calls `subscriber` for every later event, until the process ends.
pub fn subscribe(subscriber: Subscriber) {
    runner().subscribe(subscriber);
}

/// Every stored conversation, newest first.
pub fn conversations() -> Vec<ConversationSummary> {
    Store::open().map(|store| store.list()).unwrap_or_default()
}

pub fn conversation(id: &str) -> Option<Conversation> {
    Some(Store::open().ok()?.load(id)?.conversation)
}

pub fn delete(id: &str) {
    runner().cancel(id);
    crate::assistant::autonomy::forget(id);
    if let Ok(store) = Store::open() {
        store.delete(id);
    }
    {
        let mut state = runner().state();
        if state.focus.as_deref() == Some(id) {
            state.focus = None;
        }
        if state
            .last_quick
            .as_ref()
            .is_some_and(|(quick, _)| quick == id)
        {
            state.last_quick = None;
        }
    }
    runner().emit(&Event::ConversationsChanged);
}

/// The conversation Chats shows, so a quick question continues it.
pub fn set_focus(id: Option<ConversationId>) {
    runner().state().focus = id;
}

/// The conversation the next Here request should continue: a task waiting
/// for the person's answer (`NeedsInput`; the request is the answer), else a
/// Here task running now (it is steered), else the one Chats shows, else the
/// last Here conversation answered within `follow_up_seconds`, else `None`
/// (start a new one).
pub fn focus(follow_up_seconds: u32) -> Option<ConversationId> {
    runner().focus(follow_up_seconds)
}

/// `focus` without the inbox's active question: for a hold during which
/// that question came up, unheard.
pub fn focus_past_inbox(follow_up_seconds: u32) -> Option<ConversationId> {
    let runner = runner();
    let now = runner.now();
    focus_past(&runner.state(), follow_up_seconds, now)
}

/// Jobs' summaries and questions aren't read aloud for `quiet` (the person
/// said „Stopp“ or pressed Escape); what they ask for still is.
pub fn quiet_announcements(quiet: std::time::Duration) {
    voice::quiet_for(quiet, store::now_ms());
}

/// Starts the scheduler (once): every half minute it starts the scheduled
/// jobs that are due, as Background jobs; a run missed while the Mac slept
/// or Hex was closed starts once. The app calls it at launch (the runner
/// also starts it when it first runs in the app).
pub fn start_scheduler() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    // Replies from the person's phone while they're away.
    crate::assistant::phone::start();
    std::thread::Builder::new()
        .name("assistant-scheduler".into())
        .spawn(|| {
            loop {
                if current_settings().0 {
                    runner().run_due_schedules();
                }
                std::thread::sleep(Duration::from_secs(30));
            }
        })
        .ok();
}

/// Starts the scheduled jobs due now (`hex assistant schedules watch`);
/// their conversations.
pub fn run_due_schedules() -> Vec<ConversationId> {
    runner().run_due_schedules()
}

/// Lets tasks run while the assistant is off or not set up, for `hex
/// assistant`, which a person runs on purpose.
pub fn allow_without_lab() {
    WITHOUT_LAB.store(true, Ordering::Release);
}

pub(super) static WITHOUT_LAB: AtomicBool = AtomicBool::new(false);

/// Changes the settings every later task reads, in this process only (for
/// `hex assistant ask --backend …`).
pub fn adjust_settings(adjust: impl Fn(&mut AssistantSettings) + Send + Sync + 'static) {
    *ADJUST.lock().unwrap_or_else(|error| error.into_inner()) = Some(Box::new(adjust));
}

pub(super) type Adjust = Box<dyn Fn(&mut AssistantSettings) + Send + Sync>;
pub(super) static ADJUST: Mutex<Option<Adjust>> = Mutex::new(None);
