use anyhow::{Result, anyhow};

pub fn validate_choice(name: &str, value: Option<&str>, allowed: &[&str]) -> Result<()> {
    if let Some(value) = value.map(str::trim)
        && !value.is_empty()
        && !allowed.contains(&value)
    {
        return Err(anyhow!("{name} must be one of: {}", allowed.join(", ")));
    }

    Ok(())
}

pub fn validate_optional_choice(name: &str, value: Option<&str>, allowed: &[&str]) -> Result<()> {
    validate_choice(name, value, allowed)
}
