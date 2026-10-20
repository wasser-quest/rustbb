//! Input rules. Every value from a form passes through here before it reaches the database.

pub const USERNAME_MIN: usize = 3;
pub const USERNAME_MAX: usize = 32;
pub const PASSWORD_MIN: usize = 10;
pub const PASSWORD_MAX: usize = 256;
pub const EMAIL_MAX: usize = 254;
pub const TITLE_MAX: usize = 120;
pub const BODY_MAX: usize = 10_000;

/// 3 to 32 ASCII letters, digits, `_` or `-`.
pub fn username(value: &str) -> Result<&str, &'static str> {
    let len = value.chars().count();
    if !(USERNAME_MIN..=USERNAME_MAX).contains(&len) {
        return Err("The username needs 3 to 32 characters.");
    }
    if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("The username may contain only letters, digits, _ and -.");
    }
    Ok(value)
}

pub fn password(value: &str) -> Result<&str, &'static str> {
    if value.len() < PASSWORD_MIN {
        return Err("The password needs at least 10 characters.");
    }
    if value.len() > PASSWORD_MAX {
        return Err("The password is too long.");
    }
    Ok(value)
}

/// A non-empty title of at most 120 characters, without surrounding spaces.
pub fn title(value: &str) -> Result<&str, &'static str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("The title is missing.");
    }
    if trimmed.chars().count() > TITLE_MAX {
        return Err("The title is too long (at most 120 characters).");
    }
    Ok(trimmed)
}

/// A non-empty post of at most 10 000 characters.
pub fn body(value: &str) -> Result<&str, &'static str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("The message is empty.");
    }
    if trimmed.chars().count() > BODY_MAX {
        return Err("The message is too long (at most 10 000 characters).");
    }
    Ok(trimmed)
}

/// A plain ASCII address such as `name@example.org`: one `@`, a local part, and a dotted domain.
/// Nothing that could start a new mail header (no spaces or control characters) gets through.
pub fn email(value: &str) -> Result<&str, &'static str> {
    const INVALID: &str = "Enter an e-mail address like name@example.org.";
    let trimmed = value.trim();
    if trimmed.len() > EMAIL_MAX || !trimmed.is_ascii() || trimmed.chars().any(|c| c.is_ascii_control() || c == ' ') {
        return Err(INVALID);
    }
    let Some((local, domain)) = trimmed.split_once('@') else {
        return Err(INVALID);
    };
    let labels_ok = domain.split('.').all(|label| !label.is_empty());
    if local.is_empty() || local.len() > 64 || domain.contains('@') || !domain.contains('.') || !labels_ok {
        return Err(INVALID);
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e_mail_addresses_need_a_local_part_and_a_dotted_domain() {
        assert_eq!(email(" name@example.org ").unwrap(), "name@example.org");
        assert!(email("name@localhost").is_err());
        assert!(email("@example.org").is_err());
        assert!(email("name@example..org").is_err());
        assert!(email("name@example.org\r\nBcc: x@y.org").is_err());
        assert!(email("a b@example.org").is_err());
        assert!(email("a@b@example.org").is_err());
        assert!(email(&format!("{}@example.org", "a".repeat(65))).is_err());
    }

    #[test]
    fn usernames_are_short_plain_words() {
        assert!(username("ab").is_err());
        assert!(username("marvin_42").is_ok());
        assert!(username("mar vin").is_err());
        assert!(username("<script>").is_err());
        assert!(username(&"a".repeat(33)).is_err());
    }

    #[test]
    fn passwords_have_a_minimum_and_a_maximum() {
        assert!(password("short").is_err());
        assert!(password("long enough!").is_ok());
        assert!(password(&"x".repeat(257)).is_err());
    }

    #[test]
    fn titles_and_bodies_are_trimmed_and_bounded() {
        assert_eq!(title("  Hallo  ").unwrap(), "Hallo");
        assert!(title("   ").is_err());
        assert!(title(&"t".repeat(121)).is_err());
        assert!(body("  ").is_err());
        assert!(body(&"b".repeat(10_001)).is_err());
        assert_eq!(body(" Text ").unwrap(), "Text");
    }
}
