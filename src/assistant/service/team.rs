//! A job's team on the runner's side (`agent::team`): starting the helpers
//! a lead asks for, and passing its instructions on to them.

use std::sync::Arc;

use super::{Context, ConversationId, JobKind, Origin, Request, Runner, Said, State, agent, store};

impl Runner {
    /// Starts the helpers a job asked for (`agent::team`), and answers it.
    pub(crate) fn start_team(self: &Arc<Self>, ask: &agent::team::Ask) {
        let now = self.now();
        let result = {
            let mut state = self.state();
            self.team_members(&mut state, ask, now)
        };
        let started = result.as_ref().is_ok_and(|members| !members.is_empty());
        if let Err(why) = &result {
            tracing::info!(lead = ask.lead, why, "a team request was refused");
        }
        // A tell answers with the helpers told, not as started helpers.
        let told = std::mem::take(&mut self.state().told);
        (self.hooks.team_answer)(ask, result);
        // Instructions for helpers steer them, outside the state's lock;
        // a lead's words authorize only from the person's (`Said`).
        for (helper, text, said) in told {
            self.submit_as(
                Request {
                    conversation: Some(helper),
                    text,
                    origin: Origin::Background,
                    context: Context::default(),
                },
                JobKind::Ask,
                said.map_or(Said::Nobody, Said::Words),
            );
        }
        if started {
            self.refresh_mode();
            self.pump();
        }
    }

    fn team_members(
        &self,
        state: &mut State,
        ask: &agent::team::Ask,
        now: u64,
    ) -> Result<Vec<agent::team::Member>, String> {
        let lead = state
            .tasks
            .iter()
            .find(|task| task.status.conversation == ask.lead && task.running)
            .ok_or("Only a running background job can start helpers.")?;
        if lead.workspace.is_none() {
            return Err("Only a background job can start helpers; do it yourself.".into());
        }
        if lead.parent.is_some() {
            return Err("A helper can't start helpers of its own: do this part yourself.".into());
        }
        let said = lead.said.clone();
        if !ask.tell.is_empty() {
            return Ok(Self::team_tell(state, ask, said));
        }
        let working = state
            .tasks
            .iter()
            .filter(|task| task.parent.as_deref() == Some(ask.lead.as_str()))
            .count();
        if working + ask.jobs.len() > agent::team::MAX_AT_ONCE {
            return Err(format!(
                "At most {} helpers work at once and {working} still do: wait for some (hex_team_status with wait_seconds) or start fewer.",
                agent::team::MAX_AT_ONCE
            ));
        }
        let mut members = Vec::new();
        for job in &ask.jobs {
            let id = store::new_id();
            let request = Request {
                conversation: Some(id.clone()),
                text: job.task.clone(),
                origin: Origin::Background,
                context: Context::default(),
            };
            // A helper's task is the lead's words: it is authorized from the
            // person's request that led to the lead (`Said`).
            self.push_task(
                state,
                request,
                JobKind::Job {
                    title: job.title.clone(),
                },
                None,
                true,
                false,
                said.clone(),
            );
            if let Some(task) = state.tasks.last_mut() {
                task.parent = Some(ask.lead.clone());
            }
            members.push(agent::team::Member {
                id,
                title: job.title.clone(),
                task: job.task.clone(),
                state: agent::team::State::Queued,
                step: None,
                answer: None,
                started_ms: now,
            });
        }
        Ok(members)
    }

    /// Notes the steers for the helpers `ask.tell` names (they must be the
    /// lead's); the helpers told. `start_team` delivers them once the
    /// state's lock is let go.
    fn team_tell(
        state: &mut State,
        ask: &agent::team::Ask,
        said: Option<String>,
    ) -> Vec<agent::team::Member> {
        let mut told = Vec::new();
        for tell in &ask.tell {
            let text = tell.text.trim();
            if text.is_empty() {
                continue;
            }
            let helpers: Vec<(ConversationId, String)> = state
                .tasks
                .iter()
                .filter(|task| task.parent.as_deref() == Some(ask.lead.as_str()))
                .filter(|task| tell.to == "all" || task.status.conversation == tell.to)
                .map(|task| (task.status.conversation.clone(), task.status.title.clone()))
                .collect();
            for (helper, title) in helpers {
                state
                    .told
                    .push((helper.clone(), text.to_owned(), said.clone()));
                told.push(agent::team::Member {
                    id: helper,
                    title,
                    task: text.to_owned(),
                    state: agent::team::State::Running,
                    step: None,
                    answer: None,
                    started_ms: 0,
                });
            }
        }
        told
    }
}
