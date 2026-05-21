// Berger: open-source email triage daemon.
// Copyright (C) 2026 Michel-Marie Maudet
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Configuration: parsing, `${VAR}` interpolation, validation and loading
//! of `berger.yaml`.

pub mod error;

use serde::Deserialize;

use crate::config::error::ConfigError;

/// A secret string (an API token) whose `Debug` never reveals its value.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct SecretString(String);

impl SecretString {
    /// The underlying secret. Use only where the value is genuinely needed
    /// (e.g. building an `Authorization` header) — never log the result.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

/// The whole Berger configuration, parsed from `berger.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BergerConfig {
    pub bichon: BichonConfig,
    pub database: DatabaseConfig,
    pub accounts: Vec<AccountConfig>,
}

/// How to reach the upstream Bichon instance.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BichonConfig {
    pub base_url: String,
    pub api_token: SecretString,
}

/// Where the SQLite sidecar lives on disk.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DatabaseConfig {
    pub path: String,
}

/// One mail account to triage.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AccountConfig {
    pub name: String,
    pub bichon_account_id: String,
}

impl BergerConfig {
    /// Reads, interpolates, parses and validates `berger.yaml` at `path`.
    ///
    /// # Errors
    /// Returns [`ConfigError`] if the file cannot be read, an `${ENV_VAR}`
    /// is unset, the YAML is malformed, or validation fails.
    pub fn load(path: &str) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_string(),
            source,
        })?;
        Self::parse(&raw)
    }

    /// Interpolates `${VAR}` references, parses the YAML and validates it.
    ///
    /// # Errors
    /// Returns [`ConfigError`] on an unset `${ENV_VAR}`, malformed YAML, or
    /// a validation failure.
    pub fn parse(yaml: &str) -> Result<Self, ConfigError> {
        let interpolated = substitute_env_vars(yaml)?;
        let config: BergerConfig = serde_yaml_ng::from_str(&interpolated)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        fn require_non_empty(value: &str, field: &str) -> Result<(), ConfigError> {
            if value.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "`{field}` must not be empty"
                )));
            }
            Ok(())
        }

        require_non_empty(&self.bichon.base_url, "bichon.base_url")?;
        require_non_empty(self.bichon.api_token.expose(), "bichon.api_token")?;
        require_non_empty(&self.database.path, "database.path")?;

        if self.accounts.is_empty() {
            return Err(ConfigError::Validation(
                "at least one account must be configured".to_string(),
            ));
        }

        let mut seen = std::collections::HashSet::new();
        for account in &self.accounts {
            require_non_empty(&account.name, "account.name")?;
            require_non_empty(&account.bichon_account_id, "account.bichon_account_id")?;
            if !seen.insert(account.name.as_str()) {
                return Err(ConfigError::Validation(format!(
                    "duplicate account name `{}`",
                    account.name
                )));
            }
        }
        Ok(())
    }
}

/// Replaces every `${NAME}` in `text` with the value of environment
/// variable `NAME`.
fn substitute_env_vars(text: &str) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| ConfigError::Interpolation("unterminated `${` in config".to_string()))?;
        let name = &after[..end];
        if name.is_empty() {
            return Err(ConfigError::Interpolation(
                "empty `${}` placeholder in config".to_string(),
            ));
        }
        let value = std::env::var(name).map_err(|_| {
            ConfigError::Interpolation(format!("environment variable `{name}` is not set"))
        })?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_YAML: &str = r#"
bichon:
  base_url: "https://bichon.example"
  api_token: "tok-123"
database:
  path: "berger.db"
accounts:
  - name: "LINAGORA"
    bichon_account_id: "8525922389589073"
  - name: "Gmail"
    bichon_account_id: "1417038252461348"
"#;

    #[test]
    fn parses_a_valid_config() {
        let config = BergerConfig::parse(VALID_YAML).unwrap();
        assert_eq!(config.bichon.base_url, "https://bichon.example");
        assert_eq!(config.bichon.api_token.expose(), "tok-123");
        assert_eq!(config.database.path, "berger.db");
        assert_eq!(config.accounts.len(), 2);
        assert_eq!(config.accounts[0].name, "LINAGORA");
        assert_eq!(config.accounts[1].bichon_account_id, "1417038252461348");
    }

    #[test]
    fn secret_string_debug_does_not_leak_the_value() {
        let config = BergerConfig::parse(VALID_YAML).unwrap();
        assert!(!format!("{:?}", config.bichon.api_token).contains("tok-123"));
        assert!(!format!("{config:?}").contains("tok-123"));
    }

    #[test]
    fn interpolates_environment_variables() {
        // PATH is always set in the process environment.
        let yaml = VALID_YAML.replace("\"tok-123\"", "\"${PATH}\"");
        let config = BergerConfig::parse(&yaml).unwrap();
        assert_eq!(
            config.bichon.api_token.expose(),
            std::env::var("PATH").unwrap()
        );
    }

    #[test]
    fn an_unset_environment_variable_is_an_error() {
        let yaml = VALID_YAML.replace("\"tok-123\"", "\"${BERGER_DEFINITELY_UNSET_XYZ}\"");
        assert!(matches!(
            BergerConfig::parse(&yaml).unwrap_err(),
            ConfigError::Interpolation(_)
        ));
    }

    #[test]
    fn an_unterminated_interpolation_is_an_error() {
        let yaml = VALID_YAML.replace("\"tok-123\"", "\"${oops\"");
        assert!(matches!(
            BergerConfig::parse(&yaml).unwrap_err(),
            ConfigError::Interpolation(_)
        ));
    }

    #[test]
    fn malformed_yaml_is_a_parse_error() {
        assert!(matches!(
            BergerConfig::parse("bichon: [not: valid").unwrap_err(),
            ConfigError::Parse(_)
        ));
    }

    #[test]
    fn a_config_with_no_accounts_is_rejected() {
        let yaml = VALID_YAML.replace(
            "accounts:\n  - name: \"LINAGORA\"\n    bichon_account_id: \"8525922389589073\"\n  - name: \"Gmail\"\n    bichon_account_id: \"1417038252461348\"\n",
            "accounts: []\n",
        );
        assert!(matches!(
            BergerConfig::parse(&yaml).unwrap_err(),
            ConfigError::Validation(_)
        ));
    }

    #[test]
    fn duplicate_account_names_are_rejected() {
        let yaml = VALID_YAML.replace("\"Gmail\"", "\"LINAGORA\"");
        assert!(matches!(
            BergerConfig::parse(&yaml).unwrap_err(),
            ConfigError::Validation(_)
        ));
    }

    #[test]
    fn an_empty_required_field_is_rejected() {
        let yaml = VALID_YAML.replace("https://bichon.example", "");
        assert!(matches!(
            BergerConfig::parse(&yaml).unwrap_err(),
            ConfigError::Validation(_)
        ));
    }

    #[test]
    fn substitute_env_vars_leaves_plain_text_untouched() {
        let text = "no placeholders in here";
        assert_eq!(substitute_env_vars(text).unwrap(), text);
    }

    #[test]
    fn load_reads_a_config_file() {
        let path = std::env::temp_dir().join(format!("berger-cfg-{}.yaml", std::process::id()));
        std::fs::write(&path, VALID_YAML).unwrap();
        let config = BergerConfig::load(path.to_str().unwrap()).unwrap();
        assert_eq!(config.accounts.len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_reports_a_missing_file() {
        assert!(matches!(
            BergerConfig::load("/nonexistent/berger-xyz.yaml").unwrap_err(),
            ConfigError::Io { .. }
        ));
    }
}
