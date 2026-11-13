//! Projects in the runner (`assistant::projects`): which one a task runs
//! in, what is said to switch them („wechsle zu Projekt Startup“, answered
//! here and never asked of an agent), the island's offer when a request
//! seems to be about one while none is active, and what a finished task
//! adds to its project's soul.

use std::sync::Arc;

use super::{Origin, Request, Runner, TaskState};
use crate::assistant::lines::tr_named_in;
use crate::assistant::projects::ForTask;
use crate::assistant::projects::command::{self, Command};
use crate::assistant::types::{ConversationId, Event};
use crate::desktop::i18n::tr_in;

impl Runner {
    /// The project a task runs in: its conversation's (or, for a helper,
    /// its lead's), else, for a new conversation, the active one; `None`
    /// with no project, exactly as without projects.
    pub(super) fn project_for(
        &self,
        conversation: &str,
        lead: Option<&str>,
        new_conversation: bool,
        words: &str,
    ) -> Option<ForTask> {
        let projects = (self.hooks.projects)()?;
        let stored = lead
            .and_then(|lead| (self.hooks.project_of)(lead))
            .or_else(|| (self.hooks.project_of)(conversation));
        let project = match stored {
            // A conversation keeps its project, archived since or not.
            Some(id) => projects.get(&id)?,
            None if new_conversation && lead.is_none() => projects.active()?,
            None => return None,
        };
        Some(projects.for_task(&project, words))
    }

    /// Offers the project a new request seems to be about, while none is
    /// active: the island shows „In project Startup?“ with one tap.
    pub(super) fn suggest_project(self: &Arc<Self>, conversation: &str, words: &str) {
        let Some(projects) = (self.hooks.projects)() else {
            return;
        };
        if projects.active().is_some() {
            return;
        }
        if let Some(project) = projects.suggest(words) {
            self.emit(&Event::ProjectSuggested {
                conversation: conversation.to_owned(),
                id: project.id,
                name: project.name,
            });
        }
    }

    /// Adds what a finished task in project `id` named and made to its soul.
    pub(super) fn collect_for_project(
        &self,
        id: &str,
        words: &str,
        answer: &str,
        workspace: Option<&std::path::Path>,
    ) {
        let Some(projects) = (self.hooks.projects)() else {
            return;
        };
        if let Some(project) = projects.get(id) {
            let added = projects.collect(&project, words, answer, workspace);
            if !added.is_empty() {
                tracing::info!(
                    count = added.len(),
                    "added what a task learned to its project"
                );
            }
        }
    }

    /// A spoken or typed project command (`projects::command`), done and
    /// answered at once like the jobs' status; `None` when `request` isn't
    /// one (or answers a task's question).
    pub(super) fn project_command(self: &Arc<Self>, request: &Request) -> Option<ConversationId> {
        if request.origin == Origin::Background {
            return None;
        }
        let command = command::parse(&request.text)?;
        // „Kein Projekt“ may be the answer a job waits for.
        if let Some(conversation) = &request.conversation
            && self.state().tasks.iter().any(|task| {
                &task.status.conversation == conversation
                    && task.status.state == TaskState::NeedsInput
            })
        {
            return None;
        }
        let projects = (self.hooks.projects)()?;
        let language = crate::assistant::lines::language(&request.text);
        let line =
            |template: &'static str, name: &str| tr_named_in(language, template, &[("name", name)]);
        let text = match command {
            Command::Switch(name) => match projects.find(&name) {
                Some(project) => match projects.set_active(Some(&project.id)) {
                    Ok(_) => line("Project {name} is active now.", &project.name),
                    Err(error) => {
                        tracing::warn!(%error, "couldn't switch projects");
                        tr_in(language, "I couldn't change the project.").to_owned()
                    }
                },
                None => line(
                    "There's no project called {name}. Say “new project {name}” to make it.",
                    &name,
                ),
            },
            Command::Create(name) => match projects.create(&name) {
                Ok((project, made)) => match projects.set_active(Some(&project.id)) {
                    Ok(_) if made => line(
                        "New project {name} is ready and active. Its folder is in Documents › Agent-Desk › Projects.",
                        &project.name,
                    ),
                    Ok(_) => line(
                        "There already is a project {name}; it's active now.",
                        &project.name,
                    ),
                    Err(error) => {
                        tracing::warn!(%error, "couldn't switch to the new project");
                        tr_in(language, "I couldn't change the project.").to_owned()
                    }
                },
                Err(error) => {
                    tracing::warn!(%error, "couldn't make the project");
                    tr_in(language, "I couldn't make the project.").to_owned()
                }
            },
            Command::Leave => match projects.set_active(None) {
                Ok(_) => tr_in(language, "No project now.").to_owned(),
                Err(error) => {
                    tracing::warn!(%error, "couldn't leave the project");
                    tr_in(language, "I couldn't change the project.").to_owned()
                }
            },
            Command::Which => match projects.active() {
                Some(project) => line("You're in project {name}.", &project.name),
                None => tr_in(language, "No project is active.").to_owned(),
            },
        };
        let conversation = crate::assistant::store::new_id();
        let origin = if request.origin == Origin::Chat {
            Origin::Chat
        } else {
            Origin::ActionShortcut
        };
        self.emit(&Event::Started {
            conversation: conversation.clone(),
            origin,
            question: request.text.clone(),
        });
        self.emit(&Event::Answer {
            conversation: conversation.clone(),
            origin,
            text: text.clone(),
        });
        if origin == Origin::ActionShortcut {
            self.voice.announce(
                self,
                super::voice::Priority::Answer,
                &conversation,
                "",
                &text,
            );
        }
        Some(conversation)
    }
}
