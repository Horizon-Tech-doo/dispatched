use crate::error::CommandError;
use crate::state::AppState;
use serde_json::Value;

pub const MAX_SETTING_BYTES: usize = 64 * 1024;

/// A stored app setting. A value that no longer parses reads as `None`, so a
/// change in shape between builds falls back to defaults instead of failing.
pub fn get(state: &AppState, key: &str) -> Result<Option<Value>, CommandError> {
    Ok(state
        .store
        .app_setting(key)?
        .and_then(|text| serde_json::from_str(&text).ok()))
}

pub fn set(state: &AppState, key: &str, value: Option<Value>) -> Result<(), CommandError> {
    let text = value.map(|v| v.to_string());
    if text.as_ref().is_some_and(|t| t.len() > MAX_SETTING_BYTES) {
        return Err(CommandError::new("invalid", "That setting is too large to save."));
    }
    state.store.set_app_setting(key, text.as_deref())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::test_util::state;
    use serde_json::json;

    #[test]
    fn settings_round_trip_as_json() {
        let s = state();
        assert_eq!(get(&s, "ui").unwrap(), None);
        set(&s, "ui", Some(json!({"theme": "dark", "density": "compact"}))).unwrap();
        assert_eq!(get(&s, "ui").unwrap(), Some(json!({"theme": "dark", "density": "compact"})));
        set(&s, "ui", None).unwrap();
        assert_eq!(get(&s, "ui").unwrap(), None);
    }

    #[test]
    fn unparsable_stored_value_reads_as_none() {
        let s = state();
        s.store.set_app_setting("ui", Some("{not json")).unwrap();
        assert_eq!(get(&s, "ui").unwrap(), None);
    }

    #[test]
    fn oversized_values_are_rejected() {
        let s = state();
        let big = serde_json::Value::String("x".repeat(MAX_SETTING_BYTES + 1));
        let e = set(&s, "ui", Some(big)).unwrap_err();
        assert_eq!(e.kind, "invalid");
    }
}
