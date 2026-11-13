//! Starting the next request's backend ahead of it: while the person still
//! speaks (`prewarm`), and after a quick answer for the next new
//! conversation (`stand_by`, `assistant.warm_minutes`).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::{
    AssistantBackend, Context, ConversationId, Job, JobKind, Lane, Origin, Request, Runner,
    TaskWindow, Warm, focus_in, store,
};

impl Runner {
    pub(super) fn prewarm(self: &Arc<Self>, lane: Lane, window: Option<TaskWindow>) {
        self.start_ahead(lane, window, false);
    }

    /// After a quick answer: Claude Code started for the next *new* Here
    /// conversation in the window in front now, so a hold after the
    /// follow-up window still finds it up, in whichever window (the window
    /// goes through the guard's state, not the process's arguments). It
    /// idles (no tokens) until then, at most `assistant.warm_minutes`.
    pub(super) fn stand_by(self: &Arc<Self>) {
        let (enabled, settings) = (self.hooks.settings)();
        if !enabled
            || settings.warm_minutes == 0
            || settings.backend != AssistantBackend::ClaudeCode
        {
            return;
        }
        let runner = Arc::clone(self);
        std::thread::Builder::new()
            .name("assistant-stand-by".into())
            .spawn(move || {
                let window = (runner.hooks.front_window)();
                runner.start_ahead(Lane::Here, window, true);
            })
            .ok();
    }

    /// The words of a hold so far (`partial`, from the live transcript):
    /// when they ask for the web, the reserved Here conversation's own
    /// browser starts now, so its first page opens warm. Once per
    /// conversation; nothing when nothing is reserved or the Here lane has
    /// no browser.
    pub(super) fn speculate(self: &Arc<Self>, partial: &str) {
        if !crate::assistant::route::wants_web(partial, &(self.hooks.signed_in_sites)()) {
            return;
        }
        let (enabled, settings) = (self.hooks.settings)();
        if !enabled || !settings.background_browser || !settings.full_control {
            return;
        }
        let now = self.now();
        let id = {
            let mut state = self.state();
            let Some(warm) = state.warm.as_ref().filter(|warm| {
                warm.lane == Lane::Here && now.saturating_sub(warm.at_ms) < warm.for_ms
            }) else {
                return;
            };
            let id = warm.id.clone();
            if state.speculated.as_ref() == Some(&id) {
                return;
            }
            state.speculated = Some(id.clone());
            id
        };
        tracing::debug!("the hold asks for the web; starting its browser ahead");
        (self.hooks.browser_ahead)(&id, partial);
    }

    /// Reserves the next `lane` request's conversation and agent, and starts
    /// its backend: for the conversation a hold continues (`focus_in`), or
    /// for a new one when `new`.
    fn start_ahead(self: &Arc<Self>, lane: Lane, window: Option<TaskWindow>, new: bool) {
        let (enabled, settings) = (self.hooks.settings)();
        if !enabled || !settings.setup_done || settings.backend != AssistantBackend::ClaudeCode {
            // What was reserved for a backend that can't start ahead is void.
            if let Some(void) = self.state().warm.take() {
                (self.hooks.end_ahead)(&void.id);
            }
            return;
        }
        let now = self.now();
        let (conversation, id, replaced) = {
            let mut state = self.state();
            let conversation: Option<ConversationId> = match lane {
                Lane::Here if !new => focus_in(&state, settings.follow_up_seconds, now),
                _ => None,
            };
            // Speaking into a running task steers it: nothing to start.
            if conversation.as_ref().is_some_and(|conversation| {
                state
                    .tasks
                    .iter()
                    .any(|task| task.status.conversation == *conversation)
            }) {
                return;
            }
            // A Here task running when the stand-by comes: the next hold
            // speaks into it.
            if new && state.tasks.iter().any(|task| task.lane() == Lane::Here) {
                return;
            }
            if state.warm.as_ref().is_some_and(|warm| {
                warm.lane == lane
                    && warm.conversation == conversation
                    && now.saturating_sub(warm.at_ms) < warm.for_ms
            }) {
                // The same reservation serves any window: the request's own
                // goes to the guard when it runs.
                return;
            }
            let key = self.next.fetch_add(1, Ordering::Relaxed);
            let id = conversation.clone().unwrap_or_else(store::new_id);
            // The reservation this one replaces won't be taken: its process
            // ends instead of idling out its time.
            let replaced = state
                .warm
                .replace(Warm {
                    lane,
                    conversation: conversation.clone(),
                    id: id.clone(),
                    key,
                    at_ms: now,
                    for_ms: crate::assistant::agent::ahead_for(
                        &settings,
                        lane,
                        conversation.is_none(),
                    )
                    .as_millis() as u64,
                })
                .map(|old| old.id)
                .filter(|old| *old != id);
            (conversation, id, replaced)
        };
        if let Some(old) = replaced {
            (self.hooks.end_ahead)(&old);
        }
        // Everything the system prompt is made of as the real request will
        // have it (`agent::system_for`), so the process started now is the
        // one it finds: the memory note too.
        let memory = settings
            .memory
            .then(|| crate::assistant::memory::for_prompt(&(self.hooks.memory)()))
            .flatten();
        let job = Job {
            conversation: id,
            request: Request {
                conversation,
                text: String::new(),
                origin: match lane {
                    Lane::Here => Origin::ActionShortcut,
                    Lane::Background => Origin::Background,
                },
                context: Context {
                    window,
                    ..Context::default()
                },
            },
            settings,
            kind: JobKind::Ask,
            digest: None,
            workspace: None,
            authorized: Default::default(),
            memory,
            recall: None,
            project: None,
            helps: None,
        };
        let runner = Arc::clone(self);
        std::thread::Builder::new()
            .name("assistant-prewarm".into())
            .spawn(move || (runner.hooks.prewarm)(&job))
            .ok();
    }
}
