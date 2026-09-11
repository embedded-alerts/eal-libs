//! Stable contracts for Embedded Alerts.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Actor {
    pub id: String,
    pub tenant_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventEnvelope {
    pub id: String,
    pub kind: String,
    pub occurred_at: String,
    pub actor: Option<Actor>,
}

impl EventEnvelope {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.trim().is_empty() {
            return Err("event id is required");
        }
        if !valid_kind(&self.kind) {
            return Err("event kind must be lowercase and namespaced");
        }
        if self.occurred_at.trim().is_empty() {
            return Err("occurred_at is required");
        }
        Ok(())
    }
}

pub fn valid_kind(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|ch| ch == '.' || ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
        && value.contains('.')
}

pub const PRODUCT: &str = "embedded-alerts";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_namespaced_kinds() {
        assert!(valid_kind("case.created"));
        assert!(!valid_kind("CaseCreated"));
        assert!(!valid_kind("created"));
    }

    #[test]
    fn rejects_empty_disallowed_and_non_alphabetic_starts() {
        assert!(!valid_kind(""));
        assert!(!valid_kind("case.cre ated"));
        assert!(!valid_kind("1case.created"));
        assert!(valid_kind("case_2.created-now"));
    }
}
