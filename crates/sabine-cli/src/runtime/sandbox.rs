#[cfg(target_os = "linux")]
pub(super) fn profile() -> Result<String, String> {
    use sha2::{Digest, Sha256};

    let host = super::ensure_runtime_ready()?;
    let runtime_root = sabine_runtime::user_runtime_path();
    let data_root = runtime_root.parent().unwrap().parent().unwrap();
    let data_root = data_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let digest = Sha256::digest(data_root.as_os_str().as_encoded_bytes());
    let name = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let managed = format!("{}/**/sabine-host", escape(&data_root)?);
    let host = host.canonicalize().map_err(|error| error.to_string())?;
    let mut output = String::from("abi <abi/4.0>,\ninclude <tunables/global>\n\n");
    output.push_str(&attachment(&name, &managed));
    if !host.starts_with(&data_root) {
        output.push('\n');
        output.push_str(&attachment(&format!("{name}-external"), &escape(&host)?));
    }
    Ok(output)
}

#[cfg(target_os = "linux")]
fn attachment(name: &str, path: &str) -> String {
    format!("profile sabine-{name} \"{path}\" flags=(unconfined) {{\n  userns,\n}}\n")
}

#[cfg(target_os = "linux")]
fn escape(path: &std::path::Path) -> Result<String, String> {
    let path = path.to_str().ok_or("AppArmor paths must be valid UTF-8")?;
    if path.chars().any(char::is_control) {
        return Err("AppArmor paths cannot contain control characters".into());
    }
    let mut result = String::new();
    for character in path.chars() {
        if "\\\"*?[]{}^,".contains(character) {
            result.push('\\');
        }
        result.push(character);
    }
    Ok(result)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn profile() -> Result<String, String> {
    Err("AppArmor sandbox profiles are only used on Linux".into())
}
