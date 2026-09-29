use crate::diagnostics::{Diagnostic, Span};
use crate::md::{Document, FrontmatterError};
use serde::{Serialize, Serializer};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Readme,
    Howto,
    Reference,
    Runbook,
    Adr,
    Plan,
    Changelog,
    Generated,
}

impl Kind {
    pub const ALL: [Self; 8] = [
        Self::Readme,
        Self::Howto,
        Self::Reference,
        Self::Runbook,
        Self::Adr,
        Self::Plan,
        Self::Changelog,
        Self::Generated,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Readme => "readme",
            Self::Howto => "howto",
            Self::Reference => "reference",
            Self::Runbook => "runbook",
            Self::Adr => "adr",
            Self::Plan => "plan",
            Self::Changelog => "changelog",
            Self::Generated => "generated",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KindResolution {
    pub value: Option<Kind>,
    pub source: &'static str,
    pub problem: Option<String>,
}

#[derive(Clone, Debug)]
pub enum KindOutcome {
    Declared(Kind),
    Mapped(Kind),
    InvalidFrontmatter(FrontmatterError),
    GeneratedDeclaration(Span),
    Unknown { name: String, span: Span },
    Missing(Span),
}

impl KindOutcome {
    pub fn value(&self) -> Option<Kind> {
        match self {
            Self::Declared(kind) | Self::Mapped(kind) => Some(*kind),
            _ => None,
        }
    }

    pub fn resolution(&self) -> KindResolution {
        let source = match self {
            Self::Mapped(_) => "configuration",
            Self::Missing(_) => "unknown",
            _ => "frontmatter",
        };
        let problem = match self {
            Self::Declared(_) | Self::Mapped(_) => None,
            Self::InvalidFrontmatter(_) => {
                Some("Frontmatter is invalid; inspect document.frontmatter.errors.".into())
            }
            Self::GeneratedDeclaration(_) => {
                Some("The generated kind can only be assigned in configuration.".into())
            }
            Self::Unknown { name, .. } => Some(format!(
                "Unknown kind {name:?}; use one of the lowercase kinds {}.",
                declarable_kinds()
            )),
            Self::Missing(_) => Some(
                "Declare kind in frontmatter or add a matching [[kinds]] configuration entry."
                    .into(),
            ),
        };
        KindResolution {
            value: self.value(),
            source,
            problem,
        }
    }

    pub fn diagnostic(&self, document: &Document, filename: &str) -> Option<Diagnostic> {
        let (code, span, message, suggestion) = match self {
            Self::Declared(_) | Self::Mapped(_) => return None,
            Self::InvalidFrontmatter(error) => (
                "KND001", error.span,
                format!("Document kind cannot be resolved because the frontmatter is invalid: {}.", error.message.trim_end_matches('.')),
                "Correct the YAML frontmatter so its kind declaration can be read.".into(),
            ),
            Self::GeneratedDeclaration(span) => (
                "KND002", *span,
                "The generated kind is declared in frontmatter; it can only be assigned in configuration.".into(),
                "Remove this declaration and assign generated with a [[kinds]] path mapping if a tool generates this file.".into(),
            ),
            Self::Unknown { name, span } => (
                "KND002", *span, format!("Unknown document kind {name:?}."),
                format!("Use one of the lowercase kinds {} in frontmatter.", declarable_kinds()),
            ),
            Self::Missing(span) => (
                "KND001", *span,
                "Document kind is not declared and no kind mapping matches this file.".into(),
                format!("Declare kind as {} in YAML frontmatter, or add a matching [[kinds]] configuration entry.", declarable_kinds()),
            ),
        };
        Some(Diagnostic::new(
            filename,
            &document.source,
            code,
            span,
            message,
            suggestion,
        ))
    }
}

impl Serialize for KindOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.resolution().serialize(serializer)
    }
}

pub fn resolve_kind(document: &Document, mapped: Option<&str>) -> KindOutcome {
    let mut span = Span::new(0, 0);
    if let Some(frontmatter) = &document.frontmatter {
        span = frontmatter.span;
        if let Some(error) = frontmatter.errors.first() {
            return KindOutcome::InvalidFrontmatter(error.clone());
        }
        if let Some(name) = &frontmatter.kind {
            return match Kind::from_name(name) {
                Some(Kind::Generated) => KindOutcome::GeneratedDeclaration(span),
                Some(kind) => KindOutcome::Declared(kind),
                None => KindOutcome::Unknown {
                    name: name.clone(),
                    span,
                },
            };
        }
    }
    mapped.map_or(KindOutcome::Missing(span), |name| {
        KindOutcome::Mapped(Kind::from_name(name).expect("validated kind mapping"))
    })
}

fn declarable_kinds() -> String {
    let kinds: Vec<_> = Kind::ALL
        .into_iter()
        .filter(|kind| *kind != Kind::Generated)
        .map(Kind::as_str)
        .collect();
    match kinds.split_last() {
        Some((last, rest)) => format!("{}, or {last}", rest.join(", ")),
        None => String::new(),
    }
}
