//! Only an explicitly selected, authenticated Repo activates Linear.
use super::{App, Effect, SurveyId};
use crate::{
    config::{LinearWorkflow, Location, linear::LinearBinding},
    domain::{RepoId, ids::same_login},
    ordering::Scope,
    ports::LinearSurveyRequest,
};

impl App {
    pub fn selected_repo(&self) -> Option<&RepoId> {
        match &self.scope.scope {
            Scope::Repo(repo) => Some(repo),
            _ => None,
        }
    }

    pub fn linear_binding(&self) -> Option<&LinearBinding> {
        let repo = self.selected_repo()?;
        self.config
            .as_ref()
            .ok()?
            .linear
            .iter()
            .filter_map(|workflow| workflow.binding.as_ref())
            .find(|binding| binding.repo.same(repo))
    }

    pub fn linear_workflow_template(&self, repo: &RepoId) -> Option<&LinearWorkflow> {
        self.linear_templates
            .iter()
            .find(|workflow| same_login(&workflow.owner, &repo.owner))
    }

    pub fn configured_linear_env_names(&self) -> Vec<String> {
        self.linear_templates
            .iter()
            .map(|workflow| workflow.api_key_env.clone())
            .collect()
    }

    pub fn linear_request_allowed(&self, workflow: &LinearWorkflow) -> bool {
        workflow
            .binding
            .as_ref()
            .is_some_and(|binding| self.linear_binding() == Some(binding))
            && self
                .config
                .as_ref()
                .is_ok_and(|config| config.linear.iter().any(|active| active == workflow))
    }

    /// Metadata has already been validated by the connection adapter. Secrets
    /// remain behind its credential reference and never enter App state.
    pub fn activate_linear_pairing(
        &mut self,
        repo: RepoId,
        workspace_id: String,
        workspace: String,
        project_id: String,
        viewer_id: String,
        credential_ref: String,
    ) -> Vec<Effect> {
        if !self
            .selected_repo()
            .is_some_and(|selected| selected.same(&repo))
        {
            return Vec::new();
        }
        let template = self.linear_workflow_template(&repo).cloned();
        let binding = LinearBinding {
            repo: repo.clone(),
            workspace_id,
            project_id,
            viewer_id,
            credential_ref: credential_ref.clone(),
            workflow_policy: template.is_some(),
        };
        if self.linear_binding() == Some(&binding)
            && self.config.as_ref().is_ok_and(|config| {
                config
                    .linear
                    .iter()
                    .any(|w| w.api_key_env == credential_ref)
            })
        {
            return Vec::new();
        }
        self.deactivate_linear();
        let mut workflow = template.unwrap_or_else(|| LinearWorkflow {
            binding: None,
            owner: repo.owner.clone(),
            workspace: workspace.clone(),
            api_key_env: credential_ref.clone(),
            api_key_env_at: Location { line: 1, column: 1 },
            start_teams: Vec::new(),
            include_backlog: false,
            active_cycle_only: false,
            new_work_repos: Vec::new(),
            held_repos: Vec::new(),
            landing_order: Vec::new(),
            reviewers: Vec::new(),
            slack_channel: None,
            in_progress_state: None,
            release_state: None,
            parked_states: Vec::new(),
            exact_head_approval: false,
            comment_reviews_need_rereview: false,
            pending_requests_hold_merge: false,
        });
        workflow.binding = Some(binding);
        workflow.workspace = workspace;
        workflow.api_key_env = credential_ref;
        workflow.new_work_repos = vec![repo.clone()];
        let Ok(config) = &mut self.config else {
            return Vec::new();
        };
        config.linear = vec![workflow.clone()];
        self.policies = config.policies().with_linear_repo(repo);
        if let Some(now) = self.now {
            self.decide_shown(now);
        }
        if self.now.is_none() {
            return Vec::new();
        }
        let id = SurveyId(self.next_survey);
        self.next_survey += 1;
        self.linear_running = Some(id);
        vec![Effect::SurveyLinear {
            id,
            request: LinearSurveyRequest {
                workflows: vec![workflow],
            },
        }]
    }

    /// Invalidates every response and action derived from the old pairing.
    pub fn deactivate_linear(&mut self) {
        self.linear_running = None;
        self.unshown_linear = None;
        self.member_states_running = None;
        self.linear_unsaved = false;
        self.overlay = None;
        if let Ok(config) = &mut self.config {
            config.linear.clear();
            self.policies = config.policies();
        }
        if let Some(shown) = self.shown.take() {
            self.shown = Some(shown.restricted_to_linear(&[]));
        }
        if let Some(now) = self.now {
            self.decide_shown(now);
        }
    }

    /// Historical Linear Jobs cannot restore access to a revoked or different
    /// pairing. Group records contain actionable members only.
    pub(super) fn linear_job_allowed(&self, job: &crate::jobs::JobRecord) -> bool {
        use crate::jobs::JobSubject;
        let issue = match &job.subject {
            JobSubject::GitHub(_) => return true,
            JobSubject::LinearIssue { owner, issue } => crate::domain::WorkItemId::LinearIssue {
                owner: owner.clone(),
                issue: issue.clone(),
            },
            JobSubject::LandingGroup(group) => group.issue.clone(),
        };
        let Some(binding) = self.linear_binding() else {
            return false;
        };
        binding.repo.same(&job.target)
            && self.linear_issue(&issue).is_some()
            && match &job.subject {
                JobSubject::LandingGroup(group) => group
                    .members
                    .iter()
                    .all(|member| member.pr.repo.same(&binding.repo)),
                _ => true,
            }
    }

    pub(super) fn eligible_job_checks(&self) -> Vec<crate::handoff::JobCheck> {
        self.job_checks
            .iter()
            .filter(|check| self.linear_job_allowed(&check.job))
            .cloned()
            .collect()
    }

    pub(super) fn linear_cache_scope(&self) -> Option<Vec<String>> {
        let binding = self.linear_binding()?;
        let workflow = self.config.as_ref().ok()?.linear.first()?;
        Some(vec![
            binding.repo.owner.to_lowercase(),
            binding.repo.name.to_lowercase(),
            binding.workspace_id.clone(),
            binding.project_id.clone(),
            binding.viewer_id.clone(),
            workflow.api_key_env.clone(),
        ])
    }
}
