use super::*;

#[derive(Clone, Debug)]
pub(super) struct ChangeSourceRequest {
    pub(super) kind: ResolvedChangeSourceKind,
    pub(super) files: Vec<String>,
    pub(super) base: Option<String>,
    pub(super) deprecated_input_fields: Vec<String>,
}

pub(super) struct ResolvedChangeSource {
    pub(super) kind: ResolvedChangeSourceKind,
    pub(super) files: Vec<String>,
    pub(super) changes: Vec<ChangedFile>,
    pub(super) deleted_files: Vec<String>,
    pub(super) base: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BuildOperationKind {
    Build,
    Update,
}

impl BuildOperationKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Update => "update",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct BuildOperationRequest {
    pub(super) kind: BuildOperationKind,
    pub(super) change_source: Option<ChangeSourceRequest>,
    pub(super) deprecated_input_fields: Vec<String>,
}

pub(super) struct LegacyBuildUpdateFields {
    pub(super) present_fields: Vec<String>,
}
