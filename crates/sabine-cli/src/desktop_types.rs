pub fn validate(mime_types: &[String]) -> Result<(), String> {
    for mime in mime_types {
        let valid = mime.split_once('/').is_some_and(|(kind, subtype)| {
            !kind.is_empty() && !subtype.is_empty() && !subtype.contains('/')
        }) && mime
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/-._+".contains(&byte));
        if !valid {
            return Err(format!("invalid app MIME type: {mime}"));
        }
        if let Some(scheme) = mime.strip_prefix("x-scheme-handler/")
            && (!scheme
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
                || !scheme.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"+.-".contains(&byte)
                }))
        {
            return Err(format!(
                "invalid app URL scheme: {scheme}; use a lowercase URI scheme"
            ));
        }
    }
    Ok(())
}

pub fn schemes(mime_types: &[String]) -> impl Iterator<Item = &str> {
    mime_types
        .iter()
        .filter_map(|mime| mime.strip_prefix("x-scheme-handler/"))
}
