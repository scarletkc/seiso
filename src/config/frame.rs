//! Per-configuration environments and provenance-preserving TOML merging.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// An index into the evaluation's environment-frame table.
pub(super) type FrameId = usize;

/// Why an extended unit receives its path interpretation base.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FrameRelation {
    Selected,
    GoverningAncestor,
    SharedTemplate,
}

/// The environment in which one configuration unit's declarations are interpreted.
///
/// `source` resolves the unit's own `extend` directive. `policy_base` resolves
/// path-bearing settings. They differ when a shared template is inherited.
#[derive(Clone, Debug)]
pub(super) struct EnvironmentFrame {
    pub source: PathBuf,
    pub policy_base: PathBuf,
    pub extender: Option<FrameId>,
    pub relation: FrameRelation,
}

/// A TOML node whose declaration environment survives recursive overlay.
#[derive(Clone, Debug)]
pub(super) struct FramedValue {
    pub frame: FrameId,
    pub kind: FramedKind,
}

/// Structural TOML nodes keep their children, rather than flattening origins.
#[derive(Clone, Debug)]
pub(super) enum FramedKind {
    Table(BTreeMap<String, FramedValue>),
    Array(Vec<FramedValue>),
    Scalar(toml::Value),
}

impl FramedValue {
    /// Bind all declarations in one parsed unit to the same environment.
    pub fn bind(value: toml::Value, frame: FrameId) -> Self {
        let kind = match value {
            toml::Value::Table(table) => FramedKind::Table(
                table
                    .into_iter()
                    .map(|(key, value)| (key, Self::bind(value, frame)))
                    .collect(),
            ),
            toml::Value::Array(entries) => FramedKind::Array(
                entries
                    .into_iter()
                    .map(|value| Self::bind(value, frame))
                    .collect(),
            ),
            scalar => FramedKind::Scalar(scalar),
        };
        Self { frame, kind }
    }

    /// Restore ordinary TOML for existing public settings deserialization.
    pub fn into_toml(self) -> toml::Value {
        match self.kind {
            FramedKind::Table(table) => toml::Value::Table(
                table
                    .into_iter()
                    .map(|(key, value)| (key, value.into_toml()))
                    .collect(),
            ),
            FramedKind::Array(entries) => {
                toml::Value::Array(entries.into_iter().map(Self::into_toml).collect())
            }
            FramedKind::Scalar(scalar) => scalar,
        }
    }

    pub fn table(&self) -> Option<&BTreeMap<String, Self>> {
        match &self.kind {
            FramedKind::Table(table) => Some(table),
            _ => None,
        }
    }

    pub fn table_mut(&mut self) -> Option<&mut BTreeMap<String, Self>> {
        match &mut self.kind {
            FramedKind::Table(table) => Some(table),
            _ => None,
        }
    }

    pub fn array(&self) -> Option<&[Self]> {
        match &self.kind {
            FramedKind::Array(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn string(&self) -> Option<&str> {
        match &self.kind {
            FramedKind::Scalar(toml::Value::String(value)) => Some(value),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Self> {
        self.table()?.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Self> {
        self.table_mut()?.get_mut(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<Self> {
        self.table_mut()?.remove(key)
    }
}

/// Merge already-bound declarations without erasing each surviving node's frame.
pub(super) fn overlay(base: &mut FramedValue, local: FramedValue) {
    match (&mut base.kind, local) {
        (
            FramedKind::Table(base),
            FramedValue {
                kind: FramedKind::Table(local),
                ..
            },
        ) => {
            for (key, value) in local {
                match (base.get_mut(&key), value) {
                    (
                        Some(existing),
                        FramedValue {
                            kind: FramedKind::Array(mut additions),
                            ..
                        },
                    ) if matches!(
                        key.as_str(),
                        "extend-select"
                            | "extend-ignore"
                            | "extend-exclude"
                            | "extend-kinds"
                            | "extend-domains"
                            | "extend-sites"
                    ) =>
                    {
                        if let FramedKind::Array(inherited) = &mut existing.kind {
                            inherited.append(&mut additions);
                        }
                    }
                    (Some(existing), value) => overlay(existing, value),
                    (None, value) => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (_, local) => *base = local,
    }
}
