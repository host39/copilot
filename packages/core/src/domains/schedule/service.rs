//! Schedule use cases: settings with limits, previews, firing due schedules, run records, and the
//! built-in schedule tools.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::domains::project::ProjectRepository;
use crate::domains::provider::ModelRepository;
use crate::domains::schedule::builtin::{SchedulePrompt, ToolOrigin};
use crate::domains::schedule::cron::CronRule;
use crate::domains::schedule::entities::{CreatedBy, OnAsk, RunOutcome, RunRecord, Schedule};
use crate::domains::schedule::ports::{ScheduleQuery, ScheduleRepository};
use crate::ports::clock::Clock;
use crate::shared::actor::Actor;
use crate::shared::constants::MAX_SCHEDULES_PER_USER;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ChatId, ModelId, ProjectId, ScheduleId};
use crate::shared::reasoning::ReasoningLevel;

/// Runs shown by a preview.
const PREVIEW_RUNS: usize = 3;

/// Input of [`ScheduleService::create`]. New schedules are enabled.
#[derive(Clone, Debug, Default)]
pub struct NewSchedule {
    pub name: String,
    pub cron: String,
    pub timezone: String,
    pub prompt: String,
    /// `None` for a standalone schedule.
    pub project_id: Option<ProjectId>,
    pub model_id: Option<ModelId>,
    /// Used only without a project.
    pub instructions: String,
    pub reasoning: Option<ReasoningLevel>,
    pub on_ask: OnAsk,
}

/// Input of [`ScheduleService::update`]; `None` keeps a field. The target (project or
/// standalone) never changes.
#[derive(Clone, Debug, Default)]
pub struct ScheduleUpdate {
    pub name: Option<String>,
    pub cron: Option<String>,
    pub timezone: Option<String>,
    pub prompt: Option<String>,
    pub instructions: Option<String>,
    pub model_id: Option<ModelId>,
    pub reasoning: Option<Option<ReasoningLevel>>,
    pub on_ask: Option<OnAsk>,
    pub enabled: Option<bool>,
}

/// The sentence and the next runs of a rule, for the schedule form and the approval card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulePreview {
    pub description: String,
    pub timezone: String,
    pub next_runs: Vec<DateTime<Utc>>,
}

/// A run to start: a claimed due schedule or a Run now, with its owner. The chat domain turns it
/// into a prepared turn (a new chat titled `<schedule name> · <local date>`) and the scheduler
/// reports the end with [`ScheduleService::record`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Firing {
    pub schedule: Schedule,
    pub actor: Actor,
    /// When the run started (the claim, or the Run now request).
    pub at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct ScheduleService {
    schedules: Arc<dyn ScheduleRepository>,
    projects: Arc<dyn ProjectRepository>,
    models: Arc<dyn ModelRepository>,
    clock: Arc<dyn Clock>,
}

impl ScheduleService {
    pub fn new(
        schedules: Arc<dyn ScheduleRepository>,
        projects: Arc<dyn ProjectRepository>,
        models: Arc<dyn ModelRepository>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            schedules,
            projects,
            models,
            clock,
        }
    }

    pub async fn list(&self, actor: &Actor, query: ScheduleQuery) -> DomainResult<Vec<Schedule>> {
        if let Some(project) = query.project {
            self.check_project(actor, project).await?;
        }
        self.schedules.list(actor.user_id, query).await
    }

    pub async fn get(&self, actor: &Actor, id: ScheduleId) -> DomainResult<Schedule> {
        self.schedules
            .get(actor.user_id, id)
            .await?
            .ok_or_else(|| not_found(id))
    }

    /// Creates an enabled schedule after checking the rule (5 fields, time zone, minimum
    /// interval), the project and model (the caller's; the model enabled and available), and the
    /// `MAX_SCHEDULES_PER_USER` limit.
    pub async fn create(&self, actor: &Actor, input: NewSchedule) -> DomainResult<Schedule> {
        self.add(actor, input, None).await
    }

    /// A new rule moves the next run. Turning a schedule on checks the limit and the model again
    /// and clears its failure count and pause reason.
    pub async fn update(
        &self,
        actor: &Actor,
        id: ScheduleId,
        changes: ScheduleUpdate,
    ) -> DomainResult<Schedule> {
        let now = self.clock.now();
        let mut schedule = self.get(actor, id).await?;
        if let Some(name) = &changes.name {
            schedule = schedule.with_name(name)?;
        }
        if let Some(prompt) = &changes.prompt {
            schedule = schedule.with_prompt(prompt)?;
        }
        if changes.cron.is_some() || changes.timezone.is_some() {
            let cron = changes
                .cron
                .as_deref()
                .unwrap_or(schedule.rule().expression());
            let timezone = changes
                .timezone
                .as_deref()
                .unwrap_or(schedule.rule().timezone());
            let rule = CronRule::parse(cron, timezone, now)?;
            schedule = schedule.with_rule(rule, now);
        }
        if let Some(instructions) = &changes.instructions {
            schedule = schedule.with_instructions(instructions);
        }
        if let Some(model) = changes.model_id {
            self.check_model(actor, Some(model)).await?;
            schedule = schedule.with_model(Some(model));
        }
        if let Some(reasoning) = changes.reasoning {
            schedule = schedule.with_reasoning(reasoning);
        }
        if let Some(on_ask) = changes.on_ask {
            schedule = schedule.with_on_ask(on_ask);
        }
        match changes.enabled {
            Some(true) if !schedule.enabled() => {
                self.check_model(actor, schedule.model_id()).await?;
                self.check_limit(actor).await?;
                schedule = schedule.enabled_at(now);
            }
            Some(false) if schedule.enabled() => schedule = schedule.disabled(None),
            _ => {}
        }
        let schedule = schedule.touched(now);
        self.schedules.update(&schedule).await?;
        Ok(schedule)
    }

    /// The chats of its runs stay.
    pub async fn delete(&self, actor: &Actor, id: ScheduleId) -> DomainResult<()> {
        if self.schedules.delete(actor.user_id, id).await? {
            Ok(())
        } else {
            Err(not_found(id))
        }
    }

    /// The rule in plain words and its next 3 runs, checked like a saved rule.
    pub fn preview(&self, cron: &str, timezone: &str) -> DomainResult<SchedulePreview> {
        let now = self.clock.now();
        let rule = CronRule::parse(cron, timezone, now)?;
        Ok(SchedulePreview {
            description: rule.describe(),
            timezone: rule.timezone().to_owned(),
            next_runs: rule.next_n(now, PREVIEW_RUNS),
        })
    }

    /// Claims up to `limit` due schedules (the free scheduled-run slots). Each claim is a
    /// compare-and-set of `next_run_at` from the value read to the rule's next fire after `now`,
    /// so fires missed while the server was down collapse into one run, and of two schedulers
    /// only one claims a fire. Schedules of disabled users are never due.
    pub async fn fire_due(&self, now: DateTime<Utc>, limit: u32) -> DomainResult<Vec<Firing>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut firings = Vec::new();
        for (schedule, actor) in self.schedules.due(now, limit).await? {
            let (Some(expected), Some(next)) =
                (schedule.next_run_at(), schedule.rule().next_after(now))
            else {
                continue;
            };
            if self
                .schedules
                .advance(schedule.id(), expected, next, now)
                .await?
            {
                firings.push(Firing {
                    schedule: schedule.advanced(next, now),
                    actor,
                    at: now,
                });
            }
        }
        Ok(firings)
    }

    /// Run now: an unattended run exactly like a scheduled fire, without moving `next_run_at`.
    pub async fn run_now(&self, actor: &Actor, id: ScheduleId) -> DomainResult<Firing> {
        let schedule = self.get(actor, id).await?;
        Ok(Firing {
            schedule,
            actor: *actor,
            at: self.clock.now(),
        })
    }

    /// Stores how a run that started at `started_at` ended. The third failure in a row disables
    /// the schedule with the reason shown.
    pub async fn record(
        &self,
        id: ScheduleId,
        outcome: RunOutcome,
        started_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        self.schedules
            .record(id, &RunRecord::new(outcome, started_at))
            .await
    }

    /// `schedule_prompt`: creates a schedule from the calling chat's project and model (a
    /// standalone chat's schedule copies the chat instructions), marked as the assistant's.
    pub async fn schedule_prompt(
        &self,
        actor: &Actor,
        origin: &ToolOrigin,
        input: SchedulePrompt,
    ) -> DomainResult<Schedule> {
        let new = NewSchedule {
            name: input.name,
            cron: input.cron,
            timezone: input.timezone,
            prompt: input.prompt,
            project_id: origin.project_id,
            model_id: origin.model_id,
            instructions: origin.instructions.clone(),
            reasoning: None,
            on_ask: OnAsk::Deny,
        };
        self.add(actor, new, Some(origin.chat_id)).await
    }

    /// `list_schedules`: the schedules the assistant created in the calling chat's project, or
    /// from the calling chat when it has no project.
    pub async fn list_schedules(
        &self,
        actor: &Actor,
        origin: &ToolOrigin,
    ) -> DomainResult<Vec<Schedule>> {
        let query = match origin.project_id {
            Some(project) => ScheduleQuery {
                project: Some(project),
                source_chat: None,
                created_by: Some(CreatedBy::Assistant),
            },
            None => ScheduleQuery {
                project: None,
                source_chat: Some(origin.chat_id),
                created_by: Some(CreatedBy::Assistant),
            },
        };
        let mut schedules = self.schedules.list(actor.user_id, query).await?;
        if origin.project_id.is_none() {
            schedules.retain(|schedule| schedule.project_id().is_none());
        }
        Ok(schedules)
    }

    /// `cancel_schedule`: deletes one of the schedules `list_schedules` shows; any other id
    /// reads as missing.
    pub async fn cancel_schedule(
        &self,
        actor: &Actor,
        origin: &ToolOrigin,
        id: ScheduleId,
    ) -> DomainResult<Schedule> {
        let visible = self.list_schedules(actor, origin).await?;
        let schedule = visible
            .into_iter()
            .find(|schedule| schedule.id() == id)
            .ok_or_else(|| not_found(id))?;
        self.delete(actor, id).await?;
        Ok(schedule)
    }

    /// Validates and inserts a new schedule; `assistant_chat` marks it as created by the
    /// assistant in that chat.
    async fn add(
        &self,
        actor: &Actor,
        input: NewSchedule,
        assistant_chat: Option<ChatId>,
    ) -> DomainResult<Schedule> {
        let now = self.clock.now();
        let rule = CronRule::parse(&input.cron, &input.timezone, now)?;
        let mut schedule = Schedule::create(actor.user_id, &input.name, rule, &input.prompt, now)?;
        if let Some(project) = input.project_id {
            self.check_project(actor, project).await?;
        }
        self.check_model(actor, input.model_id).await?;
        self.check_limit(actor).await?;
        schedule = schedule
            .in_project(input.project_id)
            .with_instructions(&input.instructions)
            .with_model(input.model_id)
            .with_reasoning(input.reasoning)
            .with_on_ask(input.on_ask);
        if let Some(chat) = assistant_chat {
            schedule = schedule.by_assistant(chat);
        }
        self.schedules.insert(&schedule).await?;
        Ok(schedule)
    }

    async fn check_project(&self, actor: &Actor, project: ProjectId) -> DomainResult<()> {
        self.projects
            .get(actor.user_id, project)
            .await?
            .map(|_| ())
            .ok_or_else(|| DomainError::not_found("Project", project.to_string()))
    }

    /// An enabled schedule needs one of the caller's models that is enabled and available.
    async fn check_model(&self, actor: &Actor, model: Option<ModelId>) -> DomainResult<()> {
        let Some(id) = model else {
            return Err(
                DomainError::validation("Choose a model for this schedule.").with_field("modelId")
            );
        };
        let model = self
            .models
            .get(actor.user_id, id)
            .await?
            .ok_or_else(|| DomainError::not_found("Model", id.to_string()))?;
        if model.is_usable() {
            Ok(())
        } else {
            Err(DomainError::validation(
                "This model is turned off or no longer available. Choose another model.",
            )
            .with_field("modelId"))
        }
    }

    async fn check_limit(&self, actor: &Actor) -> DomainResult<()> {
        if self.schedules.count_enabled(actor.user_id).await? >= MAX_SCHEDULES_PER_USER {
            return Err(DomainError::business_rule(
                format!(
                    "You can have at most {MAX_SCHEDULES_PER_USER} schedules turned on. Turn one off first."
                ),
                "TOO_MANY_SCHEDULES",
            )
            .with_context("maxSchedules", MAX_SCHEDULES_PER_USER));
        }
        Ok(())
    }
}

fn not_found(id: ScheduleId) -> DomainError {
    DomainError::not_found("Schedule", id.to_string())
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::domains::project::{NewProject, ProjectService};
    use crate::domains::provider::{
        ModelQuery, ModelService, ModelUpdate, NewProvider, Preset, ProviderKind, ProviderService,
    };
    use crate::domains::schedule::builtin::ScheduleTool;
    use crate::domains::schedule::entities::{MODEL_REMOVED, MODEL_TURNED_OFF, RunStatus};
    use crate::shared::ids::{ChatId, ProviderId};
    use crate::shared::net::PrivateNetwork;
    use crate::testing::{
        FakeCipher, FixedClock, MemoryStore, ScriptedGateway, actor, admin, remote_model,
    };

    struct Fixture {
        service: ScheduleService,
        providers: ProviderService,
        models: ModelService,
        projects: ProjectService,
        gateway: Arc<ScriptedGateway>,
        store: Arc<MemoryStore>,
        clock: Arc<FixedClock>,
    }

    fn fixture() -> Fixture {
        let clock = Arc::new(FixedClock::default());
        let store = Arc::new(MemoryStore::new(clock.clone()));
        let gateway = Arc::new(ScriptedGateway::new());
        Fixture {
            service: ScheduleService::new(
                store.clone(),
                store.clone(),
                store.clone(),
                clock.clone(),
            ),
            providers: ProviderService::new(
                store.clone(),
                store.clone(),
                store.clone(),
                gateway.clone(),
                Arc::new(FakeCipher::new()),
                clock.clone(),
                PrivateNetwork::Admins,
            ),
            models: ModelService::new(store.clone(), store.clone(), store.clone(), clock.clone()),
            projects: ProjectService::new(
                store.clone(),
                store.clone(),
                store.clone(),
                clock.clone(),
            ),
            gateway,
            store,
            clock,
        }
    }

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, hour, minute, 0).unwrap()
    }

    /// A connection with one enabled model; returns the model id and the provider id.
    async fn model(f: &Fixture, caller: &Actor) -> (ModelId, ProviderId) {
        let provider = f
            .providers
            .create(
                caller,
                NewProvider {
                    kind: ProviderKind::OpenAi,
                    preset: Preset::OpenAi,
                    name: "OpenAI".to_owned(),
                    base_url: None,
                    api_key: Some("sk".to_owned()),
                },
            )
            .await
            .unwrap();
        f.gateway.push_models(vec![remote_model("gpt-5.6-terra")]);
        f.providers.refresh(caller, provider.id()).await.unwrap();
        let models = ModelRepository::list(
            f.store.as_ref(),
            caller.user_id,
            ModelQuery::provider(provider.id()),
        )
        .await
        .unwrap();
        (models[0].id(), provider.id())
    }

    fn daily(model: ModelId) -> NewSchedule {
        NewSchedule {
            name: "Digest".to_owned(),
            cron: "0 9 * * *".to_owned(),
            timezone: "UTC".to_owned(),
            prompt: "Summarize the news.".to_owned(),
            model_id: Some(model),
            ..NewSchedule::default()
        }
    }

    async fn project(f: &Fixture, caller: &Actor) -> ProjectId {
        let input = NewProject {
            name: "Work".to_owned(),
            ..NewProject::default()
        };
        f.projects.create(caller, input).await.unwrap().id()
    }

    fn is_not_found(error: &DomainError, entity: &str) -> bool {
        matches!(error, DomainError::EntityNotFound { entity_name, .. } if entity_name == entity)
    }

    #[tokio::test]
    async fn creates_an_enabled_schedule_due_at_the_next_fire() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let input = NewSchedule {
            instructions: " Be brief. ".to_owned(),
            reasoning: Some(ReasoningLevel::Low),
            on_ask: OnAsk::Pause,
            ..daily(model)
        };
        let schedule = f.service.create(&caller, input).await.unwrap();
        assert!(schedule.enabled());
        assert_eq!(schedule.next_run_at(), Some(at(9, 0)));
        assert_eq!(schedule.instructions(), "Be brief.");
        assert_eq!(schedule.model_id(), Some(model));
        assert_eq!(schedule.reasoning(), Some(ReasoningLevel::Low));
        assert_eq!(schedule.on_ask(), OnAsk::Pause);
        assert_eq!(schedule.created_by(), CreatedBy::User);
        assert_eq!(
            f.service.get(&caller, schedule.id()).await.unwrap(),
            schedule
        );
    }

    #[tokio::test]
    async fn a_project_schedule_keeps_no_instructions() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let project = project(&f, &caller).await;
        let input = NewSchedule {
            project_id: Some(project),
            instructions: "Ignored".to_owned(),
            ..daily(model)
        };
        let schedule = f.service.create(&caller, input).await.unwrap();
        assert_eq!(schedule.project_id(), Some(project));
        assert_eq!(schedule.instructions(), "");
        let query = ScheduleQuery {
            project: Some(project),
            ..ScheduleQuery::default()
        };
        assert_eq!(f.service.list(&caller, query).await.unwrap(), [schedule]);
    }

    #[tokio::test]
    async fn validates_the_rule_model_and_project() {
        let f = fixture();
        let alice = actor();
        let bob = actor();
        let (model, _) = model(&f, &alice).await;
        let too_often = NewSchedule {
            cron: "*/5 * * * *".to_owned(),
            ..daily(model)
        };
        let error = f.service.create(&alice, too_often).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Schedules must run at least 15 minutes apart."
        );
        let no_model = NewSchedule {
            model_id: None,
            ..daily(model)
        };
        let error = f.service.create(&alice, no_model).await.unwrap_err();
        assert_eq!(error.to_string(), "Choose a model for this schedule.");
        let error = f.service.create(&bob, daily(model)).await.unwrap_err();
        assert!(is_not_found(&error, "Model"));
        let (bob_model, _) = self::model(&f, &bob).await;
        let alice_project = project(&f, &alice).await;
        let foreign = NewSchedule {
            project_id: Some(alice_project),
            ..daily(bob_model)
        };
        let error = f.service.create(&bob, foreign).await.unwrap_err();
        assert!(is_not_found(&error, "Project"));

        let off = ModelUpdate {
            enabled: Some(false),
            ..ModelUpdate::default()
        };
        f.models.update(&alice, model, off).await.unwrap();
        let error = f.service.create(&alice, daily(model)).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "This model is turned off or no longer available. Choose another model."
        );
    }

    #[tokio::test]
    async fn limits_enabled_schedules_per_user() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let mut first = None;
        for _ in 0..MAX_SCHEDULES_PER_USER {
            let schedule = f.service.create(&caller, daily(model)).await.unwrap();
            first.get_or_insert(schedule.id());
        }
        let error = f.service.create(&caller, daily(model)).await.unwrap_err();
        assert_eq!(error.code(), "TOO_MANY_SCHEDULES");
        assert_eq!(
            error.to_string(),
            "You can have at most 20 schedules turned on. Turn one off first."
        );
        let first = first.unwrap();
        let off = ScheduleUpdate {
            enabled: Some(false),
            ..ScheduleUpdate::default()
        };
        f.service.update(&caller, first, off).await.unwrap();
        let extra = f.service.create(&caller, daily(model)).await.unwrap();
        let on = ScheduleUpdate {
            enabled: Some(true),
            ..ScheduleUpdate::default()
        };
        let error = f
            .service
            .update(&caller, first, on.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "TOO_MANY_SCHEDULES");
        let other = actor();
        let (other_model, _) = self::model(&f, &other).await;
        assert!(
            f.service.create(&other, daily(other_model)).await.is_ok(),
            "the limit is per user"
        );
        f.service.delete(&caller, extra.id()).await.unwrap();
        assert!(f.service.update(&caller, first, on).await.is_ok());
    }

    #[tokio::test]
    async fn updates_the_rule_and_toggles() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let schedule = f.service.create(&caller, daily(model)).await.unwrap();
        let changes = ScheduleUpdate {
            name: Some("Morning".to_owned()),
            cron: Some("30 7 * * 1-5".to_owned()),
            timezone: Some("Europe/Berlin".to_owned()),
            prompt: Some("Plan my day.".to_owned()),
            ..ScheduleUpdate::default()
        };
        let updated = f
            .service
            .update(&caller, schedule.id(), changes)
            .await
            .unwrap();
        assert_eq!(updated.name(), "Morning");
        assert_eq!(updated.rule().describe(), "Weekdays at 07:30");
        assert_eq!(updated.rule().timezone(), "Europe/Berlin");
        assert_eq!(
            updated.next_run_at(),
            Some(at(6, 30)),
            "07:30 CET is 06:30 UTC"
        );
        assert_eq!(updated.prompt(), "Plan my day.");

        let zone_only = ScheduleUpdate {
            timezone: Some("UTC".to_owned()),
            ..ScheduleUpdate::default()
        };
        let moved = f
            .service
            .update(&caller, schedule.id(), zone_only)
            .await
            .unwrap();
        assert_eq!(moved.rule().expression(), "30 7 * * 1-5");
        assert_eq!(moved.next_run_at(), Some(at(7, 30)));

        let off = ScheduleUpdate {
            enabled: Some(false),
            ..ScheduleUpdate::default()
        };
        let paused = f.service.update(&caller, schedule.id(), off).await.unwrap();
        assert_eq!(paused.next_run_at(), None);
        assert_eq!(paused.disabled_reason(), None);

        f.clock.advance(Duration::hours(8));
        let on = ScheduleUpdate {
            enabled: Some(true),
            ..ScheduleUpdate::default()
        };
        let resumed = f.service.update(&caller, schedule.id(), on).await.unwrap();
        assert_eq!(
            resumed.next_run_at(),
            Some(Utc.with_ymd_and_hms(2026, 1, 2, 7, 30, 0).unwrap())
        );
        assert_eq!(
            f.service.get(&caller, schedule.id()).await.unwrap(),
            resumed
        );
    }

    #[tokio::test]
    async fn previews_the_rule() {
        let f = fixture();
        let preview = f.service.preview("0 9 * * 1", "Europe/Berlin").unwrap();
        assert_eq!(preview.description, "Every Monday at 09:00");
        assert_eq!(preview.timezone, "Europe/Berlin");
        // 2026-01-05 is the first Monday; 09:00 CET is 08:00 UTC.
        let first = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        assert_eq!(
            preview.next_runs,
            [
                first,
                first + Duration::weeks(1),
                first + Duration::weeks(2)
            ]
        );
        assert!(f.service.preview("*/1 * * * *", "UTC").is_err());
        assert!(f.service.preview("0 9 * * *", "Nowhere").is_err());
    }

    #[tokio::test]
    async fn fire_due_claims_each_fire_once_and_collapses_missed_ones() {
        let f = fixture();
        let caller = admin();
        f.store.add_user(&caller);
        let (model, _) = model(&f, &caller).await;
        let schedule = f.service.create(&caller, daily(model)).await.unwrap();
        assert!(f.service.fire_due(at(8, 59), 2).await.unwrap().is_empty());

        // The server was down for three days: one run, then the next fire after now.
        let now = Utc.with_ymd_and_hms(2026, 1, 4, 12, 0, 0).unwrap();
        let firings = f.service.fire_due(now, 2).await.unwrap();
        assert_eq!(firings.len(), 1);
        let firing = &firings[0];
        assert_eq!(firing.actor, caller, "the owner's actor, with is_admin");
        assert_eq!(firing.at, now);
        let next = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
        assert_eq!(firing.schedule.next_run_at(), Some(next));
        let stored = f.service.get(&caller, schedule.id()).await.unwrap();
        assert_eq!(stored.next_run_at(), Some(next));
        assert_eq!(stored.last_run_at(), Some(now));

        assert!(
            f.service.fire_due(now, 2).await.unwrap().is_empty(),
            "claimed once"
        );
        let stale = f
            .store
            .advance(schedule.id(), at(9, 0), next, now)
            .await
            .unwrap();
        assert!(!stale, "a claim with an old next_run_at loses");
    }

    #[tokio::test]
    async fn fire_due_respects_the_limit_and_skips_disabled_users() {
        let f = fixture();
        let alice = actor();
        let bob = actor();
        let (alice_model, _) = model(&f, &alice).await;
        let (bob_model, _) = model(&f, &bob).await;
        f.service.create(&alice, daily(alice_model)).await.unwrap();
        f.service.create(&alice, daily(alice_model)).await.unwrap();
        let bobs = f.service.create(&bob, daily(bob_model)).await.unwrap();
        f.store.disable_user(alice.user_id);

        let firings = f.service.fire_due(at(10, 0), 5).await.unwrap();
        assert_eq!(firings.len(), 1);
        assert_eq!(firings[0].schedule.id(), bobs.id());
        assert!(!firings[0].actor.is_admin);
        assert!(f.service.fire_due(at(10, 0), 0).await.unwrap().is_empty());

        f.store.add_user(&alice);
        let firings = f.service.fire_due(at(10, 0), 1).await.unwrap();
        assert_eq!(firings.len(), 1, "at most `limit` claims");
        assert_eq!(f.service.fire_due(at(10, 0), 1).await.unwrap().len(), 1);
        assert!(f.service.fire_due(at(10, 0), 1).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn run_now_does_not_move_the_next_run() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let schedule = f.service.create(&caller, daily(model)).await.unwrap();
        let firing = f.service.run_now(&caller, schedule.id()).await.unwrap();
        assert_eq!(firing.actor, caller);
        assert_eq!(firing.at, f.clock.now());
        assert_eq!(firing.schedule, schedule);
        let stored = f.service.get(&caller, schedule.id()).await.unwrap();
        assert_eq!(stored.next_run_at(), Some(at(9, 0)));
        let error = f
            .service
            .run_now(&actor(), schedule.id())
            .await
            .unwrap_err();
        assert!(is_not_found(&error, "Schedule"));
    }

    #[tokio::test]
    async fn three_failed_runs_disable_the_schedule() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let schedule = f.service.create(&caller, daily(model)).await.unwrap();
        let fail = || RunOutcome::Failed("Key rejected: open provider settings.".to_owned());
        f.service
            .record(schedule.id(), fail(), at(9, 0))
            .await
            .unwrap();
        f.service
            .record(schedule.id(), RunOutcome::Ok, at(9, 1))
            .await
            .unwrap();
        for _ in 0..2 {
            f.service
                .record(schedule.id(), fail(), at(9, 2))
                .await
                .unwrap();
        }
        let stored = f.service.get(&caller, schedule.id()).await.unwrap();
        assert!(stored.enabled(), "the success reset the count");
        assert_eq!(stored.consecutive_failures(), 2);
        f.service
            .record(schedule.id(), fail(), at(9, 3))
            .await
            .unwrap();
        let stored = f.service.get(&caller, schedule.id()).await.unwrap();
        assert!(!stored.enabled());
        assert_eq!(stored.disabled_reason(), Some("Failed 3 times in a row"));
        assert_eq!(stored.last_status(), Some(RunStatus::Error));
        assert_eq!(
            stored.last_error(),
            Some("Key rejected: open provider settings.")
        );
        assert_eq!(stored.last_run_at(), Some(at(9, 3)));

        let on = ScheduleUpdate {
            enabled: Some(true),
            ..ScheduleUpdate::default()
        };
        let resumed = f.service.update(&caller, schedule.id(), on).await.unwrap();
        assert_eq!(resumed.consecutive_failures(), 0);
        assert_eq!(resumed.disabled_reason(), None);
        assert_eq!(
            resumed.last_error(),
            Some("Key rejected: open provider settings."),
            "history stays"
        );
    }

    #[tokio::test]
    async fn turning_a_model_off_or_deleting_its_provider_pauses_schedules() {
        let f = fixture();
        let caller = actor();
        let (model, provider) = model(&f, &caller).await;
        let first = f.service.create(&caller, daily(model)).await.unwrap();
        let off = ModelUpdate {
            enabled: Some(false),
            ..ModelUpdate::default()
        };
        f.models.update(&caller, model, off).await.unwrap();
        let stored = f.service.get(&caller, first.id()).await.unwrap();
        assert!(!stored.enabled());
        assert_eq!(stored.disabled_reason(), Some(MODEL_TURNED_OFF));

        let on = ModelUpdate {
            enabled: Some(true),
            ..ModelUpdate::default()
        };
        f.models.update(&caller, model, on).await.unwrap();
        let second = f.service.create(&caller, daily(model)).await.unwrap();
        let details = f.providers.get(&caller, provider).await.unwrap();
        assert_eq!(
            details.usage.schedules, 2,
            "the delete confirmation counts them"
        );
        f.providers.delete(&caller, provider).await.unwrap();
        let stored = f.service.get(&caller, second.id()).await.unwrap();
        assert!(!stored.enabled());
        assert_eq!(stored.disabled_reason(), Some(MODEL_REMOVED));
        assert_eq!(stored.model_id(), None, "ON DELETE SET NULL");
        let first = f.service.get(&caller, first.id()).await.unwrap();
        assert_eq!(
            first.disabled_reason(),
            Some(MODEL_TURNED_OFF),
            "already paused stays"
        );

        let on = ScheduleUpdate {
            enabled: Some(true),
            ..ScheduleUpdate::default()
        };
        let error = f
            .service
            .update(&caller, second.id(), on)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Choose a model for this schedule.");
    }

    #[tokio::test]
    async fn deleting_a_project_deletes_its_schedules() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let project = project(&f, &caller).await;
        let input = NewSchedule {
            project_id: Some(project),
            ..daily(model)
        };
        let schedule = f.service.create(&caller, input).await.unwrap();
        f.projects.delete(&caller, project).await.unwrap();
        assert!(f.service.get(&caller, schedule.id()).await.is_err());
    }

    #[tokio::test]
    async fn hides_other_users_schedules() {
        let f = fixture();
        let alice = actor();
        let bob = actor();
        let (model, _) = model(&f, &alice).await;
        let schedule = f.service.create(&alice, daily(model)).await.unwrap();
        assert!(
            f.service
                .list(&bob, ScheduleQuery::default())
                .await
                .unwrap()
                .is_empty()
        );
        for error in [
            f.service.get(&bob, schedule.id()).await.unwrap_err(),
            f.service
                .update(&bob, schedule.id(), ScheduleUpdate::default())
                .await
                .unwrap_err(),
            f.service.delete(&bob, schedule.id()).await.unwrap_err(),
        ] {
            assert!(is_not_found(&error, "Schedule"), "{error}");
        }
        let alice_project = project(&f, &alice).await;
        let query = ScheduleQuery {
            project: Some(alice_project),
            ..ScheduleQuery::default()
        };
        assert!(f.service.list(&bob, query).await.is_err());
    }

    fn origin(project: Option<ProjectId>, model: ModelId) -> ToolOrigin {
        ToolOrigin {
            chat_id: ChatId::generate(),
            project_id: project,
            model_id: Some(model),
            instructions: "Answer in French.".to_owned(),
        }
    }

    fn prompt(name: &str) -> SchedulePrompt {
        SchedulePrompt {
            name: name.to_owned(),
            cron: "0 8 * * *".to_owned(),
            timezone: "UTC".to_owned(),
            prompt: "Any news?".to_owned(),
        }
    }

    #[tokio::test]
    async fn schedule_prompt_fills_the_target_from_the_calling_chat() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let standalone = origin(None, model);
        let schedule = f
            .service
            .schedule_prompt(&caller, &standalone, prompt("News"))
            .await
            .unwrap();
        assert_eq!(schedule.created_by(), CreatedBy::Assistant);
        assert_eq!(schedule.source_chat_id(), Some(standalone.chat_id));
        assert_eq!(schedule.model_id(), Some(model));
        assert_eq!(schedule.project_id(), None);
        assert_eq!(schedule.instructions(), "Answer in French.");
        assert_eq!(schedule.on_ask(), OnAsk::Deny);

        let project = project(&f, &caller).await;
        let in_project = origin(Some(project), model);
        let schedule = f
            .service
            .schedule_prompt(&caller, &in_project, prompt("News"))
            .await
            .unwrap();
        assert_eq!(schedule.project_id(), Some(project));
        assert_eq!(schedule.instructions(), "");

        let mut bad = prompt("News");
        bad.cron = "* * * * *".to_owned();
        assert!(
            f.service
                .schedule_prompt(&caller, &in_project, bad)
                .await
                .is_err()
        );
        let stranger = origin(Some(project), model);
        assert!(
            f.service
                .schedule_prompt(&actor(), &stranger, prompt("x"))
                .await
                .is_err(),
            "the origin is checked against the caller"
        );
    }

    #[tokio::test]
    async fn assistant_tools_see_only_their_own_schedules_in_scope() {
        let f = fixture();
        let caller = actor();
        let (model, _) = model(&f, &caller).await;
        let project = project(&f, &caller).await;
        let here = origin(Some(project), model);
        let other_chat_same_project = origin(Some(project), model);
        let standalone = origin(None, model);
        let other_standalone = origin(None, model);

        let mine = f
            .service
            .schedule_prompt(&caller, &here, prompt("A"))
            .await
            .unwrap();
        let sibling = f
            .service
            .schedule_prompt(&caller, &other_chat_same_project, prompt("B"))
            .await
            .unwrap();
        let loose = f
            .service
            .schedule_prompt(&caller, &standalone, prompt("C"))
            .await
            .unwrap();
        f.service
            .schedule_prompt(&caller, &other_standalone, prompt("D"))
            .await
            .unwrap();
        let by_user = NewSchedule {
            project_id: Some(project),
            ..daily(model)
        };
        let by_user = f.service.create(&caller, by_user).await.unwrap();

        let names = |schedules: Vec<Schedule>| -> Vec<String> {
            schedules
                .iter()
                .map(|schedule| schedule.name().to_owned())
                .collect()
        };
        assert_eq!(
            names(f.service.list_schedules(&caller, &here).await.unwrap()),
            ["A", "B"],
            "the assistant's schedules in the project"
        );
        assert_eq!(
            names(
                f.service
                    .list_schedules(&caller, &standalone)
                    .await
                    .unwrap()
            ),
            ["C"],
            "the assistant's schedules from this chat"
        );

        let error = f
            .service
            .cancel_schedule(&caller, &here, by_user.id())
            .await
            .unwrap_err();
        assert!(
            is_not_found(&error, "Schedule"),
            "the user's own schedule is out of reach"
        );
        assert!(
            f.service
                .cancel_schedule(&caller, &standalone, mine.id())
                .await
                .is_err()
        );
        let cancelled = f
            .service
            .cancel_schedule(&caller, &here, sibling.id())
            .await
            .unwrap();
        assert_eq!(cancelled.id(), sibling.id());
        assert!(f.service.get(&caller, sibling.id()).await.is_err());
        assert!(
            f.service
                .cancel_schedule(&caller, &standalone, loose.id())
                .await
                .is_ok()
        );
        assert!(ScheduleTool::List.spec().alias.is_builtin());
    }
}
