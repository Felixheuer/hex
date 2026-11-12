//! Starting the next request's backend while the person still speaks.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::{
    AssistantBackend, Context, Job, JobKind, Lane, Origin, PREWARM_FOR_MS, Request, Runner,
    Session, TaskWindow, Warm, focus_in, store,
};

impl Runner {
    pub(super) fn prewarm(self: &Arc<Self>, lane: Lane, window: Option<TaskWindow>) {
        let (enabled, settings) = (self.hooks.settings)();
        if !enabled || !settings.setup_done || settings.backend != AssistantBackend::ClaudeCode {
            return;
        }
        let now = self.now();
        let (conversation, id, key, session) = {
            let mut state = self.state();
            let conversation = match lane {
                Lane::Here => focus_in(&state, settings.follow_up_seconds, now),
                Lane::Background => None,
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
            if state.warm.as_ref().is_some_and(|warm| {
                warm.lane == lane
                    && warm.conversation == conversation
                    && warm.window == window
                    && now.saturating_sub(warm.at_ms) < PREWARM_FOR_MS
            }) {
                return;
            }
            let session = match &state.session {
                Session::On(id) => Some(id.clone()),
                _ => None,
            };
            // A Background task joins a session that starts with it; its
            // arguments can't be known ahead.
            if lane == Lane::Background && settings.control_desktop && session.is_none() {
                return;
            }
            let key = self.next.fetch_add(1, Ordering::Relaxed);
            let id = conversation.clone().unwrap_or_else(store::new_id);
            state.warm = Some(Warm {
                lane,
                conversation: conversation.clone(),
                id: id.clone(),
                key,
                window: window.clone(),
                at_ms: now,
            });
            (conversation, id, key, session)
        };
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
            agent_id: format!("hex-{key}"),
            desktop_session: session.filter(|_| settings.control_desktop),
            settings,
            kind: JobKind::Ask,
            digest: None,
            workspace: None,
        };
        let runner = Arc::clone(self);
        std::thread::Builder::new()
            .name("assistant-prewarm".into())
            .spawn(move || (runner.hooks.prewarm)(&job))
            .ok();
    }
}
