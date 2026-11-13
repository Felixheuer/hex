//! A job's team on the runner's side (`agent::team`): starting the helpers
//! a lead asks for, and passing its instructions on to them.

use std::sync::Arc;

use super::{Context, JobKind, Origin, Request, Runner, State, TaskState, agent, store};

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
        (self.hooks.team_answer)(ask, result);
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
            return Ok(self.team_tell(state, ask, said));
        }
        let working = state
            .tasks
            .iter()
            .filter(|task| task.parent.as_deref() == Some(ask.lead.as_str()))
            .count();
        // As many as this computer runs well at once start now, all in one
        // go; the lead hears which didn't and starts them as these finish.
        let limit = agent::team::max_at_once();
        let room = crate::assistant::capacity::room_for(ask.jobs.len(), working, limit);
        if room == 0 {
            return Err(format!(
                "At most {limit} helpers work at once on this computer and {working} still do: wait for some (hex_team_status with wait_seconds) or start fewer."
            ));
        }
        let mut members = Vec::new();
        for job in ask.jobs.iter().take(room) {
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
            // Its main browser starts now, while its agent does.
            crate::assistant::browser::prewarm_for(&id, &job.task);
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

    /// Passes the lead's instructions on to the helpers `ask.tell` names
    /// (they must be its own): a working helper is steered, a queued one
    /// gets the words added to its task; the helpers told. Done right here
    /// rather than as a request, so a tell never answers a helper's
    /// question or confirmation (those wait on the person), is never routed
    /// to another task by its words, and says nothing aloud.
    fn team_tell(
        &self,
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
            let keys: Vec<u64> = state
                .tasks
                .iter()
                .filter(|task| task.parent.as_deref() == Some(ask.lead.as_str()))
                .filter(|task| tell.to == "all" || task.status.conversation == tell.to)
                .filter(|task| task.status.state != TaskState::NeedsInput)
                .filter(|task| !task.control.steered())
                .map(|task| task.key)
                .collect();
            for key in keys {
                let Some(task) = state.tasks.iter_mut().find(|task| task.key == key) else {
                    continue;
                };
                let (helper, title) = (task.status.conversation.clone(), task.status.title.clone());
                if task.running {
                    let kind = JobKind::Steer {
                        doing: task.status.step.clone(),
                        steps: task.steps.iter().cloned().collect(),
                    };
                    let context = Context {
                        screenshot: None,
                        ..task.request.context.clone()
                    };
                    task.control.steer();
                    let request = Request {
                        conversation: Some(helper.clone()),
                        text: text.to_owned(),
                        origin: Origin::Background,
                        context,
                    };
                    self.push_task(state, request, kind, None, false, false, said.clone());
                    if let Some(steered) = state.tasks.last_mut() {
                        steered.parent = Some(ask.lead.clone());
                        steered.status.title = title.clone();
                    }
                } else {
                    task.request.text = format!("{}\n\n{text}", task.request.text.trim_end());
                }
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
