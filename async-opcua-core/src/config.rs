// OPCUA for Rust
// SPDX-License-Identifier: MPL-2.0
// Copyright (C) 2017-2024 Adam Lock

//! Common utilities for configuration files in both the server and client.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::result::Result;

use serde;
use serde_saphyr;

use opcua_types::{ApplicationDescription, ApplicationType, LocalizedText, UAString};

/// Error returned from saving or loading config objects.
#[derive(Debug)]
pub enum ConfigError {
    /// Configuration is invalid, with a list of validation errors.
    ConfigInvalid(Vec<String>),
    /// Reading or writing file failed.
    IO(std::io::Error),
    /// Failed to serialize config object.
    YamlSerialize(serde_saphyr::SerializeError),
    /// Failed to deserialize config object.
    YamlDeserialize(serde_saphyr::DeserializeError),
}

impl From<std::io::Error> for ConfigError {
    fn from(value: std::io::Error) -> Self {
        Self::IO(value)
    }
}

impl From<serde_saphyr::SerializeError> for ConfigError {
    fn from(value: serde_saphyr::SerializeError) -> Self {
        Self::YamlSerialize(value)
    }
}

impl From<serde_saphyr::DeserializeError> for ConfigError {
    fn from(value: serde_saphyr::DeserializeError) -> Self {
        Self::YamlDeserialize(value)
    }
}

#[cfg(feature = "env_expansion")]
mod env_expansion;

/// Options shared by every config load.
fn load_options() -> serde_saphyr::Options {
    #[allow(unused_mut)]
    let mut options = serde_saphyr::options! {
        duplicate_keys: serde_saphyr::DuplicateKeyPolicy::PassThrough,
        strict_booleans: true,
    };
    #[cfg(feature = "env_expansion")]
    {
        options.property_syntax = serde_saphyr::PropertySyntax::Custom(
            serde_saphyr::PropertyCustomFormatter::new(env_expansion::expand_env),
        );
    }
    options
}

/// A trait that handles the loading / saving and validity of configuration information for a
/// client and/or server.
pub trait Config: serde::Serialize {
    /// Save the configuration object to a file.
    fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Err(e) = self.validate() {
            return Err(ConfigError::ConfigInvalid(e));
        }
        let s = serde_saphyr::to_string(&self)?;
        let mut f = File::create(path)?;
        f.write_all(s.as_bytes())?;
        Ok(())
    }

    /// Load the configuration object from the given path.
    ///
    /// With the `env_expansion` feature, `$NAME` / `${NAME...}` references in values are
    /// expanded from the environment first.
    fn load<A>(path: &Path) -> Result<A, ConfigError>
    where
        for<'de> A: Config + serde::Deserialize<'de>,
    {
        let mut f = File::open(path)?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        Ok(serde_saphyr::from_str_with_options(&s, load_options())?)
    }

    /// Validate the config struct, returning a list of validation errors if it fails.
    fn validate(&self) -> Result<(), Vec<String>>;

    /// Get the application name.
    fn application_name(&self) -> UAString;

    /// Get the application URI.
    fn application_uri(&self) -> UAString;

    /// Get the configured product URI.
    fn product_uri(&self) -> UAString;

    /// Get the application type.
    fn application_type(&self) -> ApplicationType;

    /// Get the registered discovery URLs for this application.
    fn discovery_urls(&self) -> Option<Vec<UAString>> {
        None
    }

    /// Create an application description for the configured application.
    fn application_description(&self) -> ApplicationDescription {
        ApplicationDescription {
            application_uri: self.application_uri(),
            application_name: LocalizedText::new("", self.application_name().as_ref()),
            application_type: self.application_type(),
            product_uri: self.product_uri(),
            gateway_server_uri: UAString::null(),
            discovery_profile_uri: UAString::null(),
            discovery_urls: self.discovery_urls(),
        }
    }
}
