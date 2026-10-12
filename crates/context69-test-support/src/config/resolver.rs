//! Selection of the one private configuration source for backend tests.

use std::path::Path;

use super::{ConfigError, Endpoint};
use crate::{REPO_ENV_DATABASE_URL_KEY, REPO_ENV_FILE_NAME, TEST_DATABASE_URL_VAR};

/// Where the selected target came from. Both sources are designated by the
/// operator; an ambient `DATABASE_URL` inherited from the shell is never one of
/// them because resolution never consults it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetSource {
    /// [`crate::TEST_DATABASE_URL_VAR`] in the runner process environment.
    EnvironmentOverride,
    /// [`crate::REPO_ENV_DATABASE_URL_KEY`] in the repository `.env` file.
    RepositoryEnvFile,
}

impl TargetSource {
    /// Short label used in run summaries. It never carries a value.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::EnvironmentOverride => TEST_DATABASE_URL_VAR,
            Self::RepositoryEnvFile => "repository .env DATABASE_URL",
        }
    }
}

/// A validated target plus the source that selected it.
#[derive(Debug)]
pub struct ResolvedTarget {
    endpoint: Endpoint,
    source: TargetSource,
}

impl ResolvedTarget {
    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    #[must_use]
    pub fn source(&self) -> TargetSource {
        self.source
    }
}

/// Resolve the test database from an explicit override, falling back to the
/// `DATABASE_URL` entry of `env_file`.
///
/// # Errors
///
/// Returns the sanitized [`ConfigError`] category describing why no single
/// target could be selected. A failure is never reported as success.
pub fn resolve(
    override_value: Option<&str>,
    env_file: &Path,
) -> Result<ResolvedTarget, ConfigError> {
    if let Some(raw) = override_value {
        if raw.trim().is_empty() {
            return Err(ConfigError::OverrideEmpty);
        }
        let endpoint = Endpoint::parse(raw).map_err(ConfigError::OverrideMalformed)?;
        return Ok(ResolvedTarget {
            endpoint,
            source: TargetSource::EnvironmentOverride,
        });
    }

    let endpoint = env_file_value(env_file)?;
    Ok(ResolvedTarget {
        endpoint,
        source: TargetSource::RepositoryEnvFile,
    })
}

/// Resolve the test database from the runner process environment and the
/// repository `.env` file.
///
/// # Errors
///
/// Propagates [`resolve`], using [`crate::repo_root`] to locate the `.env`
/// file so the result does not depend on the caller's working directory.
pub fn resolve_default() -> Result<ResolvedTarget, ConfigError> {
    let env_file = crate::repo_root().join(REPO_ENV_FILE_NAME);
    let override_value = std::env::var(TEST_DATABASE_URL_VAR).ok();
    resolve(override_value.as_deref(), &env_file)
}

/// Read `DATABASE_URL` from `env_file` without mutating the process
/// environment. The first declaration in the file wins, matching dotenvy's own
/// precedence. Values are never rendered on any path.
fn env_file_value(env_file: &Path) -> Result<Endpoint, ConfigError> {
    let entries = match dotenvy::from_path_iter(env_file) {
        Ok(entries) => entries,
        Err(error) if error.not_found() => return Err(ConfigError::TargetNotConfigured),
        Err(_) => return Err(ConfigError::EnvFileUnreadable),
    };

    let mut declared: Option<String> = None;
    for entry in entries {
        match entry {
            Ok((key, value)) => {
                if key == REPO_ENV_DATABASE_URL_KEY && declared.is_none() {
                    declared = Some(value);
                }
            }
            Err(_) => return Err(ConfigError::EnvFileMalformed),
        }
    }

    match declared {
        Some(value) if !value.trim().is_empty() => {
            Endpoint::parse(&value).map_err(ConfigError::EnvFileValueMalformed)
        }
        _ => Err(ConfigError::TargetNotConfigured),
    }
}
