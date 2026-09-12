use std::{fs, path::Path};

pub(super) struct CargoPackage {
    package: toml::Table,
    workspace: toml::Table,
}

impl CargoPackage {
    pub fn read(path: &Path) -> Result<Self, String> {
        let manifest = read_manifest(path)?;
        let package = manifest
            .get("package")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| format!("missing [package] in {}", path.display()))?
            .clone();
        let directory = path
            .parent()
            .ok_or("Cargo manifest has no parent directory")?;
        let workspace = if let Some(root) = package.get("workspace").and_then(toml::Value::as_str) {
            read_manifest(&directory.join(root).join("Cargo.toml"))?
        } else {
            let mut found = toml::Table::new();
            for ancestor in directory.ancestors() {
                let candidate = ancestor.join("Cargo.toml");
                if candidate.is_file() {
                    let manifest = read_manifest(&candidate)?;
                    if manifest.contains_key("workspace") {
                        found = manifest;
                        break;
                    }
                }
            }
            found
        };
        let workspace = workspace
            .get("workspace")
            .and_then(|value| value.get("package"))
            .and_then(toml::Value::as_table)
            .cloned()
            .unwrap_or_default();
        Ok(Self { package, workspace })
    }

    fn value(&self, field: &str) -> Option<&toml::Value> {
        let value = self.package.get(field);
        if value
            .and_then(|value| value.get("workspace"))
            .and_then(toml::Value::as_bool)
            == Some(true)
        {
            self.workspace.get(field)
        } else {
            value
        }
    }

    pub fn strings(&self, field: &str) -> Vec<String> {
        self.value(field)
            .and_then(toml::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn string(&self, field: &str) -> Result<String, String> {
        self.value(field)
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("missing Cargo package {field}"))
    }
}

fn read_manifest(path: &Path) -> Result<toml::Table, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    toml::from_str(&text).map_err(|error| format!("could not parse {}: {error}", path.display()))
}
