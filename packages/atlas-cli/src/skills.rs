//! Agent skill guides served by the installed Atlas binary.
//!
//! Guides live in `skill-data/` and are embedded at build time, so the content
//! always matches the installed version. The managed `AGENTS.md` block is only
//! a pointer at `atlas skills get core`; it never carries the guide body. Add or
//! edit guide content here, not in the instruction block or the discovery stub.

/// One skill guide shipped with this Atlas version.
#[derive(Clone, Copy, Debug)]
pub struct Skill {
    pub name: &'static str,
    pub summary: &'static str,
    pub content: &'static str,
}

pub const CORE: Skill = Skill {
    name: "core",
    summary: "Atlas-first workflow, tool map, escalation order, and trust checks",
    content: include_str!("../skill-data/core/SKILL.md"),
};

const SKILLS: &[Skill] = &[CORE];

/// Every guide this Atlas version serves, in display order.
pub fn all() -> &'static [Skill] {
    SKILLS
}

/// Resolve one guide by exact name.
pub fn get(name: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|skill| skill.name == name)
}

/// Comma-separated names for error and help output.
pub fn names() -> String {
    SKILLS
        .iter()
        .map(|skill| skill.name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_guide_is_embedded() {
        assert!(CORE.content.contains("# Atlas core workflow"));
        assert!(CORE.content.contains("atlas skills get core"));
        assert!(
            CORE.content
                .contains("Do not treat this instruction block as exhaustive inventory.")
        );
    }

    #[test]
    fn skill_names_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for skill in all() {
            assert!(
                seen.insert(skill.name),
                "duplicate skill name: {}",
                skill.name
            );
        }
    }

    #[test]
    fn get_resolves_known_and_rejects_unknown() {
        assert_eq!(get("core").map(|skill| skill.name), Some("core"));
        assert!(get("missing").is_none());
    }
}
