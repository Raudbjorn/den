//! Project and identity constraints remain enforced at the network boundary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use den::{
    config::{LinearWorkflow, linear::LinearBinding},
    domain::{LinearIssueId, RepoId},
    linear::{CredentialError, LinearApi, write::LinearWrites},
    ports::{
        LinearIssueRead, LinearIssueRequest, LinearSource, LinearSurveyRequest, LinearWriteOutcome,
        LinearWriter,
    },
};
use serde_json::json;
use std::sync::Arc;
use support::{
    build::now,
    fakes::{FixedClock, ScriptedLinear, SharedLinear},
    harness::linear_workflow,
    linear_replies::{KEY, issues_page, ok, source, viewer_body},
};
fn workflow() -> LinearWorkflow {
    let mut workflow = linear_workflow("acme", "acme");
    workflow.binding = Some(LinearBinding {
        repo: RepoId::new("acme", "app"),
        workspace_id: "workspace-1".into(),
        project_id: "project-1".into(),
        viewer_id: "user-me".into(),
        credential_ref: "connection-1".into(),
        workflow_policy: true,
    });
    workflow
}
fn viewer() -> serde_json::Value {
    let mut body = viewer_body("acme");
    body["data"]["organization"]["id"] = json!("workspace-1");
    body
}
#[test]
fn every_assigned_and_start_priority_query_is_project_scoped() {
    let transport = Arc::new(ScriptedLinear::new(|request| {
        Ok(ok(match request.operation.as_str() {
            "LinearViewer" => viewer(),
            "LinearTeamStates" => {
                json!({"data":{"teams":{"nodes":[{"id":"team-1","key":"TEAM","name":"Team","states":{"nodes":[],"pageInfo":{"hasNextPage":false}}}],"pageInfo":{"hasNextPage":false}}}})
            }
            "LinearAssignedIssues" => issues_page(vec![], None),
            "LinearStartTeamIssues" => json!({"data":{"issues":{"nodes":[],"pageInfo":{
                "hasNextPage":request.variables["filter"]["priority"].is_null(),"endCursor":"cursor"
            }}}}),
            other => panic!("unexpected operation {other}"),
        }))
    }));
    let mut workflow = workflow();
    workflow.start_teams = vec!["TEAM".into()];
    let survey = source(transport.clone(), Arc::default()).survey(&LinearSurveyRequest {
        workflows: vec![workflow],
    });
    assert!(!survey.authentication_failed);
    let requests: Vec<_> = transport
        .requests()
        .into_iter()
        .filter(|r| r.operation == "LinearAssignedIssues" || r.operation == "LinearStartTeamIssues")
        .collect();
    assert_eq!(
        requests.len(),
        7,
        "assigned, initial Start page, and all five priority buckets"
    );
    for request in requests {
        assert_eq!(
            request.variables["filter"]["project"]["id"]["eq"],
            "project-1"
        );
    }
}
#[test]
fn a_different_viewer_or_workspace_disables_the_connection_before_issue_reads() {
    for workspace_changed in [true, false] {
        let transport = Arc::new(ScriptedLinear::new(move |_| {
            let mut body = viewer();
            if workspace_changed {
                body["data"]["organization"]["id"] = json!("other-workspace");
            } else {
                body["data"]["viewer"]["id"] = json!("other-user");
            }
            Ok(ok(body))
        }));
        let survey = source(transport.clone(), Arc::default()).survey(&LinearSurveyRequest {
            workflows: vec![workflow()],
        });
        assert!(survey.authentication_failed);
        assert!(survey.part.issues.is_empty());
        assert_eq!(transport.requests().len(), 1);
    }
}
#[test]
fn moving_an_issue_out_of_the_project_prevents_reads_and_writes() {
    let transport = Arc::new(ScriptedLinear::new(|request| {
        assert!(
            !request.query.contains("mutation"),
            "no mutation may be sent"
        );
        Ok(ok(if request.operation == "LinearViewer" {
            viewer()
        } else {
            json!({"data":{
                "organization":{"id":"workspace-1","urlKey":"acme"},
                "issue":{"id":"issue-1","project":{"id":"other-project"},"assignee":null,
                "state":{"id":"todo","name":"Todo","type":"unstarted"},
                "team":{"id":"team-1","states":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}
            }})
        }))
    }));
    let request = LinearIssueRequest {
        workflow: workflow(),
        issue: LinearIssueId::new("acme", "issue-1"),
        identifier: "TEAM-1".into(),
    };
    let api = source(transport.clone(), Arc::default());
    assert!(
        matches!(api.read_issue(&request),LinearIssueRead::Failed {error} if error.contains("paired Linear project"))
    );
    let connect = transport.clone();
    let writer = LinearWrites::with(
        Box::new(|_| Some(KEY.into())),
        Box::new(move |_| Box::new(SharedLinear(connect.clone()))),
        Arc::new(FixedClock(now())),
    );
    assert!(
        matches!(writer.assign_to_viewer(&request),LinearWriteOutcome::Failed {error} if error.contains("paired Linear project"))
    );
}
#[test]
fn temporary_credential_failure_is_distinct_from_lost_authentication() {
    for authentication_failed in [false, true] {
        let api =
            LinearApi::new(Arc::new(FixedClock(now()))).with_credentials(Box::new(move |_| {
                Err(CredentialError {
                    message: "unavailable".into(),
                    authentication_failed,
                })
            }));
        let survey = api.survey(&LinearSurveyRequest {
            workflows: vec![workflow()],
        });
        assert_eq!(survey.authentication_failed, authentication_failed);
        assert!(survey.part.issues.is_empty());
    }
}
