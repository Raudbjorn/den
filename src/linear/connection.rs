//! Explicit Repo/project pairings; only credential references are persisted here.
mod oauth;

use super::transport::{ApiKey, HttpsTransport, LinearRequest, LinearTransport};
use crate::{
    domain::RepoId,
    process::{self, Invocation},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Public identifier of den's registered native OAuth application (not a secret).
pub const DEFAULT_OAUTH_CLIENT_ID: &str = "b2e5c2ddd4dc433bfa9d275c4de3f687";

const STATE_LIMIT: u64 = 1024 * 1024;
const SECRET_LIMIT: usize = 16 * 1024;
const STORE_TIMEOUT: Duration = Duration::from_secs(20);
const PROJECT_PAGES: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ConnectionError {
    message: String,
    authentication_failed: bool,
}
impl ConnectionError {
    /// Definite loss of credentials/identity; transient I/O must preserve partial facts.
    pub fn is_authentication_failure(&self) -> bool {
        self.authentication_failed
    }
}
fn error(message: &str) -> ConnectionError {
    ConnectionError {
        message: message.to_owned(),
        authentication_failed: false,
    }
}
fn auth_error(message: &str) -> ConnectionError {
    ConnectionError {
        message: message.to_owned(),
        authentication_failed: true,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialSource {
    Environment(String),
    Keyring,
    OAuth,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub workspace_id: String,
    pub workspace: String,
    pub viewer_id: String,
    pub viewer_name: String,
    pub source: CredentialSource,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoPairing {
    pub repo: RepoId,
    pub connection_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub project_name: String,
}
#[derive(Debug, Clone)]
pub struct ValidatedPairing {
    pub pairing: RepoPairing,
    pub connection: Connection,
    pub key: ApiKey,
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    connections: BTreeMap<String, Connection>,
    pairings: BTreeMap<String, RepoPairing>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Secret {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_at: Option<u64>,
}
trait Credentials: Send + Sync {
    fn read(&self, id: &str) -> Result<Secret, ConnectionError>;
    fn write(&self, id: &str, secret: &Secret) -> Result<(), ConnectionError>;
    fn remove(&self, id: &str) -> Result<(), ConnectionError>;
}
struct Keyring;
impl Keyring {
    fn command(args: &[&str], input: &[u8]) -> Result<Vec<u8>, ConnectionError> {
        let result = process::run_invocation(
            &Invocation::new("secret-tool").args(args.iter().copied()),
            input,
            STORE_TIMEOUT,
        )
        .map_err(|_| {
            error("Could not access the OS credential store; unlock it or use an environment key")
        })?;
        if !result.status.success() || result.output_left_open || result.stdout.len() > SECRET_LIMIT
        {
            return Err(error(
                "OS credential store operation failed; unlock it or use an environment key",
            ));
        }
        Ok(result.stdout)
    }
}
impl Credentials for Keyring {
    fn read(&self, id: &str) -> Result<Secret, ConnectionError> {
        let bytes = Self::command(
            &["lookup", "application", "den", "linear-connection", id],
            &[],
        ).map_err(|_| auth_error("Stored Linear credentials are unavailable; unlock the credential store or reconnect"))?;
        serde_json::from_slice(&bytes).map_err(|_| {
            auth_error("Stored Linear credentials are unavailable or invalid; reconnect")
        })
    }
    fn write(&self, id: &str, secret: &Secret) -> Result<(), ConnectionError> {
        let bytes =
            serde_json::to_vec(secret).map_err(|_| error("Could not encode Linear credentials"))?;
        if bytes.len() > SECRET_LIMIT {
            return Err(error("Linear credentials exceed the storage limit"));
        }
        Self::command(
            &[
                "store",
                "--label=den Linear connection",
                "application",
                "den",
                "linear-connection",
                id,
            ],
            &bytes,
        )?;
        Ok(())
    }
    fn remove(&self, id: &str) -> Result<(), ConnectionError> {
        Self::command(
            &["clear", "application", "den", "linear-connection", id],
            &[],
        )?;
        Ok(())
    }
}

type Connect = dyn Fn(ApiKey) -> Box<dyn LinearTransport> + Send + Sync;
type Refresh = dyn Fn(&str, &Secret) -> Result<Secret, ConnectionError> + Send + Sync;
#[derive(Clone)]
pub struct ConnectionManager {
    state_dir: PathBuf,
    oauth_client_id: Option<String>,
    credentials: Arc<dyn Credentials>,
    connect: Arc<Connect>,
    refresh: Arc<Refresh>,
}
impl ConnectionManager {
    pub fn new(state_dir: PathBuf, oauth_client_id: Option<String>) -> Self {
        Self {
            state_dir,
            oauth_client_id: Some(
                oauth_client_id.unwrap_or_else(|| DEFAULT_OAUTH_CLIENT_ID.to_owned()),
            ),
            credentials: Arc::new(Keyring),
            connect: Arc::new(|key| Box::new(HttpsTransport::new(key))),
            refresh: Arc::new(oauth::refresh),
        }
    }
    fn lock(&self) -> Result<File, ConnectionError> {
        fs::create_dir_all(&self.state_dir)
            .map_err(|_| error("Could not create den state directory"))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.state_dir.join("linear-connections.lock"))
            .map_err(|_| error("Could not open Linear connection lock"))?;
        let deadline = Instant::now() + STORE_TIMEOUT;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                _ => return Err(error("Linear connection store is busy or unavailable")),
            }
        }
    }
    fn load(&self) -> Result<State, ConnectionError> {
        let path = self.state_dir.join("linear-connections.json");
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
            Err(_) => return Err(error("Could not read Linear connections")),
        };
        let mut bytes = Vec::new();
        file.take(STATE_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error("Could not read Linear connections"))?;
        if bytes.len() as u64 > STATE_LIMIT {
            return Err(error("Linear connections file exceeds size limit"));
        }
        serde_json::from_slice(&bytes).map_err(|_| error("Linear connections file is invalid"))
    }
    fn save(&self, state: &State) -> Result<(), ConnectionError> {
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|_| error("Could not encode Linear connections"))?;
        if bytes.len() as u64 > STATE_LIMIT {
            return Err(error("Too many Linear connections or pairings"));
        }
        let temporary = self.state_dir.join("linear-connections.json.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|_| error("Could not save Linear connections"))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| error("Could not save Linear connections"))?;
        fs::rename(temporary, self.state_dir.join("linear-connections.json"))
            .map_err(|_| error("Could not replace Linear connections"))
    }
    pub fn connections(&self) -> Result<Vec<Connection>, ConnectionError> {
        Ok(self.load()?.connections.into_values().collect())
    }
    pub fn pairing(&self, repo: &RepoId) -> Result<Option<RepoPairing>, ConnectionError> {
        Ok(self.load()?.pairings.remove(&repo_key(repo)))
    }
    pub fn detected_environment_names(configured: &[String]) -> Vec<String> {
        super::cli_credentials::environment_names(configured.iter().map(String::as_str))
    }
    pub fn connect_key(&self, key: ApiKey) -> Result<Connection, ConnectionError> {
        validate_key(key.expose())?;
        self.establish(key, CredentialSource::Keyring, None, None)
    }
    pub fn connect_env(&self, name: &str) -> Result<Connection, ConnectionError> {
        if name.is_empty()
            || name.len() > 128
            || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || name.as_bytes()[0].is_ascii_digit()
        {
            return Err(error("Invalid credential environment variable name"));
        }
        let key = environment_key(name)?;
        self.establish(
            key,
            CredentialSource::Environment(name.to_owned()),
            None,
            None,
        )
    }
    pub fn connect_cli(&self) -> Result<Connection, ConnectionError> {
        let key = super::cli_credentials::read_token().map_err(|_| {
            auth_error("Linear CLI credentials are unavailable; authenticate with the CLI first")
        })?;
        self.establish(key, CredentialSource::Cli, None, None)
    }
    pub fn login(&self, cancel: &AtomicBool) -> Result<Connection, ConnectionError> {
        let client=self.oauth_client_id.as_deref().filter(|s| !s.trim().is_empty()).ok_or_else(|| error("Browser login is unavailable: the den OAuth application client ID has not been configured; use an API key"))?;
        let secret = oauth::login(client, cancel)?;
        self.establish(
            ApiKey::new(format!("Bearer {}", secret.access_token)),
            CredentialSource::OAuth,
            Some(secret),
            Some(cancel),
        )
    }
    fn establish(
        &self,
        key: ApiKey,
        source: CredentialSource,
        secret: Option<Secret>,
        cancel: Option<&AtomicBool>,
    ) -> Result<Connection, ConnectionError> {
        check_cancel(cancel)?;
        let mut connection = self.identity(&key)?;
        check_cancel(cancel)?;
        connection.id = oauth::nonce()?;
        connection.source = source;
        let _lock = self.lock()?;
        let mut state = self.load()?;
        check_cancel(cancel)?;
        if matches!(
            connection.source,
            CredentialSource::Keyring | CredentialSource::OAuth
        ) {
            self.credentials.write(
                &connection.id,
                &secret.unwrap_or(Secret {
                    access_token: key.expose().to_owned(),
                    refresh_token: None,
                    expires_at: None,
                }),
            )?;
        }
        state
            .connections
            .insert(connection.id.clone(), connection.clone());
        if let Err(failure) = check_cancel(cancel).and_then(|()| self.save(&state)) {
            if matches!(
                connection.source,
                CredentialSource::Keyring | CredentialSource::OAuth
            ) {
                self.credentials.remove(&connection.id).map_err(|_| error("Connection was not saved, but credential cleanup failed; remove the den entry from your OS credential store"))?;
            }
            return Err(failure);
        }
        Ok(connection)
    }
    /// Resolves credentials without changing process environment or logging their value.
    pub fn authorization(&self, id: &str) -> Result<ApiKey, ConnectionError> {
        let _lock = self.lock()?;
        let connection = self
            .load()?
            .connections
            .remove(id)
            .ok_or_else(|| auth_error("Linear connection no longer exists"))?;
        let key = match &connection.source {
            CredentialSource::Environment(name) => environment_key(name),
            CredentialSource::Cli => super::cli_credentials::read_token()
                .map_err(|_| auth_error("Linear CLI authentication is unavailable; reconnect")),
            CredentialSource::Keyring => {
                let secret = self.credentials.read(id)?;
                validate_key(&secret.access_token)?;
                Ok(ApiKey::new(secret.access_token))
            }
            CredentialSource::OAuth => {
                let mut secret = self.credentials.read(id)?;
                if secret
                    .expires_at
                    .is_none_or(|expires| expires <= oauth::now().saturating_add(60))
                {
                    let client = self
                        .oauth_client_id
                        .as_deref()
                        .ok_or_else(|| auth_error("OAuth application client ID is unavailable"))?;
                    secret = (self.refresh)(client, &secret)?;
                    self.credentials.write(id, &secret).map_err(|_| {
                        auth_error(
                            "Could not securely save refreshed Linear credentials; reconnect",
                        )
                    })?;
                }
                validate_key(&secret.access_token)?;
                Ok(ApiKey::new(format!("Bearer {}", secret.access_token)))
            }
        }?;
        drop(_lock);
        let current = self.identity(&key)?;
        if connection.workspace_id != current.workspace_id
            || connection.viewer_id != current.viewer_id
        {
            return Err(auth_error(
                "Linear account or workspace changed; reconnect and confirm pairing again",
            ));
        }
        if !self.load()?.connections.contains_key(id) {
            return Err(auth_error("Linear was disconnected during authentication"));
        }
        Ok(key)
    }
    /// Credentials for a still-paired Repo. Another den process can unpair or
    /// replace the project while this process retains an in-memory binding.
    pub fn authorization_for(
        &self,
        repo: &RepoId,
        connection_id: &str,
        project_id: &str,
    ) -> Result<ApiKey, ConnectionError> {
        let pairing = self
            .pairing(repo)?
            .filter(|pairing| {
                pairing.connection_id == connection_id && pairing.project_id == project_id
            })
            .ok_or_else(|| {
                auth_error("The Repo is no longer paired with this Linear connection and project")
            })?;
        let key = self.authorization(connection_id)?;
        if self.pairing(repo)?.as_ref() != Some(&pairing) {
            return Err(auth_error("Repo pairing changed during authentication"));
        }
        Ok(key)
    }
    fn query(
        &self,
        key: &ApiKey,
        operation: &str,
        query: &str,
        variables: Value,
    ) -> Result<Value, ConnectionError> {
        let reply = (self.connect)(key.clone())
            .post(&LinearRequest {
                operation: operation.to_owned(),
                query: query.to_owned(),
                variables,
            })
            .map_err(|_| error("Could not reach Linear; retry the connection"))?;
        match reply.status {
            401 | 403 => return Err(auth_error("Linear rejected authentication; reconnect")),
            429 => return Err(error("Linear is rate limited; retry later")),
            200..=299 => {}
            _ => return Err(error("Linear returned an unsuccessful response")),
        }
        let body: Value = serde_json::from_slice(&reply.body)
            .map_err(|_| error("Linear returned an invalid response"))?;
        if let Some(errors) = body
            .get("errors")
            .and_then(Value::as_array)
            .filter(|errors| !errors.is_empty())
        {
            if errors.iter().any(|item| {
                matches!(
                    item["extensions"]["code"].as_str(),
                    Some("AUTHENTICATION_ERROR" | "FORBIDDEN")
                )
            }) {
                return Err(auth_error("Linear rejected authentication; reconnect"));
            }
            return Err(error(
                "Linear could not complete the request; check account permissions",
            ));
        }
        body.get("data")
            .cloned()
            .ok_or_else(|| error("Linear returned no data"))
    }
    fn identity(&self, key: &ApiKey) -> Result<Connection, ConnectionError> {
        let data = self.query(
            key,
            "DenConnectionViewer",
            "query DenConnectionViewer { viewer { id name } organization { id urlKey } }",
            json!({}),
        )?;
        Ok(Connection {
            id: String::new(),
            workspace_id: field(&data["organization"], "id")?,
            workspace: field(&data["organization"], "urlKey")?,
            viewer_id: field(&data["viewer"], "id")?,
            viewer_name: field(&data["viewer"], "name")?,
            source: CredentialSource::Keyring,
        })
    }
    fn verified_connection(&self, id: &str) -> Result<(Connection, ApiKey), ConnectionError> {
        let stored = self
            .load()?
            .connections
            .remove(id)
            .ok_or_else(|| auth_error("Linear connection no longer exists"))?;
        let key = self.authorization(id)?;
        Ok((stored, key))
    }
    pub fn projects(&self, id: &str) -> Result<Vec<Project>, ConnectionError> {
        let (_, key) = self.verified_connection(id)?;
        let mut after: Option<String> = None;
        let mut projects = Vec::new();
        for _ in 0..PROJECT_PAGES {
            let data=self.query(&key,"DenConnectionProjects","query DenConnectionProjects($after: String) { projects(first: 50, after: $after) { nodes { id name } pageInfo { hasNextPage endCursor } } }",json!({"after":after}))?;
            let page = &data["projects"];
            let nodes = page["nodes"]
                .as_array()
                .ok_or_else(|| error("Linear returned an invalid project list"))?;
            for node in nodes {
                projects.push(Project {
                    id: field(node, "id")?,
                    name: field(node, "name")?,
                });
            }
            if page["pageInfo"]["hasNextPage"].as_bool() == Some(false) {
                return Ok(projects);
            }
            let next = field(&page["pageInfo"], "endCursor")?;
            if after.as_ref() == Some(&next) {
                return Err(error("Linear repeated its project cursor"));
            }
            after = Some(next);
        }
        Err(error(
            "Linear project listing exceeded its page limit; narrow workspace access before pairing",
        ))
    }
    fn verify_project(&self, key: &ApiKey, project_id: &str) -> Result<Project, ConnectionError> {
        let data = self.query(
            key,
            "DenConnectionProject",
            "query DenConnectionProject($id: String!) { project(id: $id) { id name } }",
            json!({"id":project_id}),
        )?;
        let project = Project {
            id: field(&data["project"], "id")?,
            name: field(&data["project"], "name")?,
        };
        if project.id != project_id {
            return Err(error("Linear returned a different project"));
        }
        Ok(project)
    }
    pub fn pair(
        &self,
        repo: &RepoId,
        id: &str,
        project: &Project,
    ) -> Result<RepoPairing, ConnectionError> {
        let (connection, key) = self.verified_connection(id)?;
        let project = self.verify_project(&key, &project.id)?;
        let pairing = RepoPairing {
            repo: repo.folded(),
            connection_id: id.to_owned(),
            workspace_id: connection.workspace_id,
            project_id: project.id,
            project_name: project.name,
        };
        let _lock = self.lock()?;
        let mut state = self.load()?;
        if !state.connections.contains_key(id) {
            return Err(auth_error("Linear was disconnected during pairing"));
        }
        state.pairings.insert(repo_key(repo), pairing.clone());
        self.save(&state)?;
        Ok(pairing)
    }
    pub fn validate(&self, repo: &RepoId) -> Result<Option<ValidatedPairing>, ConnectionError> {
        let Some(pairing) = self.pairing(repo)? else {
            return Ok(None);
        };
        let (connection, key) = self.verified_connection(&pairing.connection_id)?;
        if pairing.workspace_id != connection.workspace_id {
            return Err(auth_error(
                "Pairing workspace changed; pair this Repo again",
            ));
        }
        self.verify_project(&key, &pairing.project_id)?;
        if self.pairing(repo)?.as_ref() != Some(&pairing) {
            return Err(auth_error("Repo pairing changed during validation"));
        }
        Ok(Some(ValidatedPairing {
            pairing,
            connection,
            key,
        }))
    }
    pub fn unpair(&self, repo: &RepoId) -> Result<(), ConnectionError> {
        let _lock = self.lock()?;
        let mut state = self.load()?;
        state.pairings.remove(&repo_key(repo));
        self.save(&state)
    }
    pub fn disconnect(&self, id: &str) -> Result<(), ConnectionError> {
        let _lock = self.lock()?;
        let mut state = self.load()?;
        let connection = state
            .connections
            .remove(id)
            .ok_or_else(|| auth_error("Linear connection no longer exists"))?;
        state
            .pairings
            .retain(|_, pairing| pairing.connection_id != id);
        self.save(&state)?;
        if matches!(
            connection.source,
            CredentialSource::Keyring | CredentialSource::OAuth
        ) {
            self.credentials.remove(id)?;
        }
        Ok(())
    }
}
fn check_cancel(cancel: Option<&AtomicBool>) -> Result<(), ConnectionError> {
    if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
        return Err(error("Linear login canceled"));
    }
    Ok(())
}
fn repo_key(repo: &RepoId) -> String {
    format!(
        "{}/{}",
        repo.owner.to_ascii_lowercase(),
        repo.name.to_ascii_lowercase()
    )
}
fn field(value: &Value, name: &str) -> Result<String, ConnectionError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 1024)
        .map(str::to_owned)
        .ok_or_else(|| error("Linear returned incomplete connection data"))
}
fn validate_key(value: &str) -> Result<(), ConnectionError> {
    if value.is_empty()
        || value.len() > SECRET_LIMIT
        || value.bytes().any(|c| c.is_ascii_control() || c == b' ')
    {
        return Err(auth_error("Linear credential is empty or invalid"));
    }
    Ok(())
}
fn environment_key(name: &str) -> Result<ApiKey, ConnectionError> {
    let value = std::env::var(name)
        .map_err(|_| auth_error("Linear credential environment variable is unavailable"))?;
    validate_key(&value)?;
    Ok(ApiKey::new(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linear::transport::{HttpReply, TransportError};
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    #[derive(Default)]
    struct MemoryCredentials(Mutex<BTreeMap<String, Secret>>);
    impl Credentials for MemoryCredentials {
        fn read(&self, id: &str) -> Result<Secret, ConnectionError> {
            self.0
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .ok_or_else(|| error("missing test credential"))
        }
        fn write(&self, id: &str, s: &Secret) -> Result<(), ConnectionError> {
            self.0.lock().unwrap().insert(id.into(), s.clone());
            Ok(())
        }
        fn remove(&self, id: &str) -> Result<(), ConnectionError> {
            self.0.lock().unwrap().remove(id);
            Ok(())
        }
    }
    struct FakeTransport(Arc<AtomicBool>);
    impl LinearTransport for FakeTransport {
        fn post(&self, r: &LinearRequest) -> Result<HttpReply, TransportError> {
            let viewer = if self.0.load(Ordering::Acquire) {
                "changed-viewer"
            } else {
                "viewer"
            };
            let data = match r.operation.as_str() {
                "DenConnectionViewer" => {
                    json!({"viewer":{"id":viewer,"name":"A viewer"},"organization":{"id":"workspace-id","urlKey":"workspace"}})
                }
                "DenConnectionProjects" => {
                    json!({"projects":{"nodes":[{"id":"project","name":"Project"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}})
                }
                "DenConnectionProject" => {
                    json!({"project":{"id":r.variables["id"],"name":"Project"}})
                }
                _ => panic!("unexpected operation"),
            };
            Ok(HttpReply {
                status: 200,
                headers: vec![],
                body: serde_json::to_vec(&json!({"data":data})).unwrap(),
            })
        }
    }
    fn fixture() -> (tempfile::TempDir, ConnectionManager, Arc<AtomicBool>) {
        let temp = tempfile::tempdir().unwrap();
        let changed = Arc::new(AtomicBool::new(false));
        let c = changed.clone();
        let manager = ConnectionManager {
            state_dir: temp.path().to_owned(),
            oauth_client_id: None,
            credentials: Arc::new(MemoryCredentials::default()),
            connect: Arc::new(move |_| Box::new(FakeTransport(c.clone()))),
            refresh: Arc::new(oauth::refresh),
        };
        (temp, manager, changed)
    }
    #[test]
    fn connecting_does_not_pair_and_metadata_never_contains_key() {
        let (temp, m, _) = fixture();
        let c = m.connect_key(ApiKey::new("private-test-key")).unwrap();
        assert_eq!(m.pairing(&RepoId::new("Owner", "Repo")).unwrap(), None);
        let saved = fs::read_to_string(temp.path().join("linear-connections.json")).unwrap();
        assert!(!saved.contains("private-test-key"));
        assert_eq!(m.authorization(&c.id).unwrap().expose(), "private-test-key");
    }
    #[test]
    fn pairing_is_repo_specific_case_insensitive_and_project_scoped() {
        let (_temp, m, _) = fixture();
        let c = m.connect_key(ApiKey::new("key")).unwrap();
        let projects = m.projects(&c.id).unwrap();
        let repo = RepoId::new("Owner", "Repo");
        m.pair(&repo, &c.id, &projects[0]).unwrap();
        let paired = m.validate(&RepoId::new("owner", "repo")).unwrap().unwrap();
        assert_eq!(paired.pairing.project_id, "project");
        assert!(
            m.validate(&RepoId::new("Owner", "Sibling"))
                .unwrap()
                .is_none()
        );
        m.pair(
            &repo,
            &c.id,
            &Project {
                id: "another-project".into(),
                name: "Another".into(),
            },
        )
        .unwrap();
        assert_eq!(
            m.validate(&repo).unwrap().unwrap().pairing.project_id,
            "another-project"
        );
    }
    #[test]
    fn changed_viewer_rejects_reuse_of_existing_pairing_and_authorization() {
        let (_temp, m, changed) = fixture();
        let c = m.connect_key(ApiKey::new("key")).unwrap();
        let repo = RepoId::new("owner", "repo");
        m.pair(
            &repo,
            &c.id,
            &Project {
                id: "project".into(),
                name: "Project".into(),
            },
        )
        .unwrap();
        changed.store(true, Ordering::Release);
        assert!(
            m.authorization(&c.id)
                .unwrap_err()
                .is_authentication_failure()
        );
        assert!(m.validate(&repo).unwrap_err().is_authentication_failure());
    }
    #[test]
    fn unpair_is_local_but_disconnect_removes_all_matching_pairings() {
        let (_temp, m, _) = fixture();
        let c = m.connect_key(ApiKey::new("key")).unwrap();
        let other = m.connect_key(ApiKey::new("other-key")).unwrap();
        let project = Project {
            id: "project".into(),
            name: "Project".into(),
        };
        let a = RepoId::new("owner", "a");
        let b = RepoId::new("owner", "b");
        let d = RepoId::new("owner", "d");
        m.pair(&a, &c.id, &project).unwrap();
        m.pair(&b, &c.id, &project).unwrap();
        m.pair(&d, &other.id, &project).unwrap();
        m.unpair(&a).unwrap();
        assert!(m.pairing(&a).unwrap().is_none());
        assert!(m.pairing(&b).unwrap().is_some());
        m.disconnect(&c.id).unwrap();
        assert!(m.pairing(&b).unwrap().is_none());
        assert!(m.pairing(&d).unwrap().is_some());
        assert!(m.authorization(&c.id).is_err());
    }
    #[test]
    fn browser_login_with_empty_client_id_fails_before_opening_browser() {
        let (_temp, mut m, _) = fixture();
        m.oauth_client_id = Some(String::new());
        let result = m.login(&AtomicBool::new(false));
        assert!(result.unwrap_err().to_string().contains("client ID"));
    }
    #[test]
    fn native_app_identifier_is_used_unless_explicitly_overridden() {
        let temp = tempfile::tempdir().unwrap();
        let manager = ConnectionManager::new(temp.path().to_path_buf(), None);
        assert_eq!(
            manager.oauth_client_id.as_deref(),
            Some(DEFAULT_OAUTH_CLIENT_ID)
        );
        let other = ConnectionManager::new(temp.path().to_path_buf(), Some("other-app".into()));
        assert_eq!(other.oauth_client_id.as_deref(), Some("other-app"));
    }
    #[test]
    fn malformed_state_is_not_replaced_and_credentials_are_validated() {
        let (temp, m, _) = fixture();
        fs::write(temp.path().join("linear-connections.json"), "invalid").unwrap();
        assert!(m.connections().is_err());
        for key in ["", "line\nbreak", "contains space"] {
            assert!(validate_key(key).is_err());
        }
    }
    #[test]
    fn credential_failures_are_distinct_from_transient_service_errors() {
        struct Reply(Result<HttpReply, TransportError>);
        impl LinearTransport for Reply {
            fn post(&self, _: &LinearRequest) -> Result<HttpReply, TransportError> {
                self.0.clone()
            }
        }
        let (_temp, mut manager, _) = fixture();
        for (reply, auth_failure) in [
            (Ok(HttpReply { status: 401, ..Default::default() }), true),
            (Ok(HttpReply { status: 403, ..Default::default() }), true),
            (Ok(HttpReply { status: 429, ..Default::default() }), false),
            (Ok(HttpReply { status: 503, ..Default::default() }), false),
            (Ok(HttpReply { status: 200, body: br#"{"errors":[{"message":"secret-not-for-display","extensions":{"code":"AUTHENTICATION_ERROR"}}]}"#.to_vec(), ..Default::default() }), true),
            (Err(TransportError::Network { detail: "secret-not-for-display".into() }), false),
        ] {
            manager.connect = Arc::new(move |_| Box::new(Reply(reply.clone())));
            let failure = manager.identity(&ApiKey::new("test-secret")).unwrap_err();
            assert_eq!(failure.is_authentication_failure(), auth_failure);
            assert!(!format!("{failure:?}").contains("secret-not-for-display"));
        }
    }
    fn expire_oauth(manager: &mut ConnectionManager, id: &str) {
        manager.oauth_client_id = Some("public-test-client".into());
        let mut state = manager.load().unwrap();
        state.connections.get_mut(id).unwrap().source = CredentialSource::OAuth;
        manager.save(&state).unwrap();
        manager
            .credentials
            .write(
                id,
                &Secret {
                    access_token: "old-access".into(),
                    refresh_token: Some("old-refresh".into()),
                    expires_at: Some(0),
                },
            )
            .unwrap();
    }

    #[test]
    fn concurrent_authorization_rotates_once_and_atomically_reuses_new_bundle() {
        use std::sync::{Barrier, atomic::AtomicUsize};
        let (_temp, mut manager, _) = fixture();
        let connection = manager.connect_key(ApiKey::new("initial-key")).unwrap();
        expire_oauth(&mut manager, &connection.id);
        let count = Arc::new(AtomicUsize::new(0));
        let updates = count.clone();
        manager.refresh = Arc::new(move |client, old| {
            assert_eq!(client, "public-test-client");
            assert_eq!(old.refresh_token.as_deref(), Some("old-refresh"));
            updates.fetch_add(1, Ordering::SeqCst);
            Ok(Secret {
                access_token: "new-access".into(),
                refresh_token: Some("new-refresh".into()),
                expires_at: Some(oauth::now() + 3600),
            })
        });
        let ready = Arc::new(Barrier::new(3));
        let tasks = (0..2)
            .map(|_| {
                let manager = manager.clone();
                let id = connection.id.clone();
                let ready = ready.clone();
                std::thread::spawn(move || {
                    ready.wait();
                    manager.authorization(&id)
                })
            })
            .collect::<Vec<_>>();
        ready.wait();
        for task in tasks {
            assert_eq!(
                task.join().unwrap().unwrap(),
                ApiKey::new("Bearer new-access")
            );
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let stored = manager.credentials.read(&connection.id).unwrap();
        assert_eq!(stored.access_token, "new-access");
        assert_eq!(stored.refresh_token.as_deref(), Some("new-refresh"));
    }

    #[test]
    fn transient_refresh_failure_keeps_old_bundle_but_unsaved_rotation_disables_auth() {
        struct FailedWrites(Arc<dyn Credentials>);
        impl Credentials for FailedWrites {
            fn read(&self, id: &str) -> Result<Secret, ConnectionError> {
                self.0.read(id)
            }
            fn write(&self, _: &str, _: &Secret) -> Result<(), ConnectionError> {
                Err(error("test store unavailable"))
            }
            fn remove(&self, id: &str) -> Result<(), ConnectionError> {
                self.0.remove(id)
            }
        }
        let (_temp, mut manager, _) = fixture();
        let connection = manager.connect_key(ApiKey::new("initial-key")).unwrap();
        expire_oauth(&mut manager, &connection.id);
        manager.refresh = Arc::new(|_, _| Err(error("temporary service failure")));
        assert!(
            !manager
                .authorization(&connection.id)
                .unwrap_err()
                .is_authentication_failure()
        );
        assert_eq!(
            manager
                .credentials
                .read(&connection.id)
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("old-refresh")
        );
        manager.refresh = Arc::new(|_, _| {
            Ok(Secret {
                access_token: "new-access".into(),
                refresh_token: Some("new-refresh".into()),
                expires_at: Some(oauth::now() + 3600),
            })
        });
        manager.credentials = Arc::new(FailedWrites(manager.credentials.clone()));
        assert!(
            manager
                .authorization(&connection.id)
                .unwrap_err()
                .is_authentication_failure()
        );
    }

    #[test]
    fn cancellation_after_identity_read_does_not_persist_login() {
        struct CancelWhileReading(Arc<AtomicBool>);
        impl LinearTransport for CancelWhileReading {
            fn post(&self, request: &LinearRequest) -> Result<HttpReply, TransportError> {
                self.0.store(true, Ordering::Release);
                FakeTransport(Arc::new(AtomicBool::new(false))).post(request)
            }
        }
        let (_temp, mut manager, _) = fixture();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        manager.connect = Arc::new(move |_| Box::new(CancelWhileReading(signal.clone())));
        assert!(
            manager
                .establish(
                    ApiKey::new("key"),
                    CredentialSource::OAuth,
                    None,
                    Some(&cancel)
                )
                .is_err()
        );
        assert!(manager.connections().unwrap().is_empty());
    }
    #[test]
    fn authorization_for_requires_the_current_repo_project_pairing() {
        let (_temp, manager, _) = fixture();
        let connection = manager.connect_key(ApiKey::new("key")).unwrap();
        let repo = RepoId::new("owner", "repo");
        let project = Project {
            id: "project".into(),
            name: "Project".into(),
        };
        manager.pair(&repo, &connection.id, &project).unwrap();
        assert!(
            manager
                .authorization_for(&RepoId::new("OWNER", "REPO"), &connection.id, &project.id)
                .is_ok()
        );
        manager.unpair(&repo).unwrap();
        assert!(
            manager.authorization(&connection.id).is_ok(),
            "unpairing retains the account"
        );
        assert!(
            manager
                .authorization_for(&repo, &connection.id, &project.id)
                .unwrap_err()
                .is_authentication_failure()
        );
        manager
            .pair(
                &repo,
                &connection.id,
                &Project {
                    id: "replacement".into(),
                    name: "Replacement".into(),
                },
            )
            .unwrap();
        assert!(
            manager
                .authorization_for(&repo, &connection.id, &project.id)
                .unwrap_err()
                .is_authentication_failure()
        );
        assert!(
            manager
                .authorization_for(&repo, &connection.id, "replacement")
                .is_ok()
        );
    }

    #[test]
    fn authorization_for_rechecks_pairing_after_identity_io() {
        struct RemovePairing(ConnectionManager, RepoId);
        impl LinearTransport for RemovePairing {
            fn post(&self, request: &LinearRequest) -> Result<HttpReply, TransportError> {
                self.0.unpair(&self.1).unwrap();
                FakeTransport(Arc::new(AtomicBool::new(false))).post(request)
            }
        }
        let (_temp, mut manager, _) = fixture();
        let connection = manager.connect_key(ApiKey::new("key")).unwrap();
        let repo = RepoId::new("owner", "repo");
        manager
            .pair(
                &repo,
                &connection.id,
                &Project {
                    id: "project".into(),
                    name: "Project".into(),
                },
            )
            .unwrap();
        let parallel = manager.clone();
        let changed_repo = repo.clone();
        manager.connect =
            Arc::new(move |_| Box::new(RemovePairing(parallel.clone(), changed_repo.clone())));
        assert!(
            manager
                .authorization_for(&repo, &connection.id, "project")
                .unwrap_err()
                .is_authentication_failure()
        );
        assert_eq!(
            manager.connections().unwrap().len(),
            1,
            "only the pairing was removed"
        );
    }
}
