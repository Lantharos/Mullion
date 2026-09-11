use sabine_runtime::{
    RuntimeConfig, RuntimeInfo, RuntimeInstallProgress, ensure_runtime,
    install_user_runtime_with_progress, resolve_runtime,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::types::{
    AppManifest, REGISTRY_VERSION, RegisteredApp, ServiceError, ServiceResult, unix_timestamp,
    version_is_newer,
};

#[derive(Clone, Debug)]
pub struct SabineService {
    pub(crate) root: PathBuf,
    pub(crate) runtime: RuntimeConfig,
}

impl Default for SabineService {
    fn default() -> Self {
        Self::new(crate::types::service_data_dir())
    }
}

impl SabineService {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            runtime: RuntimeConfig::default(),
        }
    }

    pub fn with_runtime(mut self, runtime: RuntimeConfig) -> Self {
        self.runtime = runtime;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn register(&self, manifest: AppManifest) -> ServiceResult<RegisteredApp> {
        manifest.validate()?;
        let _lock = RegistryLock::acquire(&self.root)?;
        let mut registry = self.load_registry()?;
        if let Some(message) =
            incompatibility_message(&manifest, crate::install::installed_system_compatibility())
        {
            return Err(ServiceError::IncompatibleApp {
                app_id: manifest.id,
                message,
            });
        }
        if let Some(existing) = registry.apps.get(&manifest.id)
            && version_is_newer(&existing.manifest.version, &manifest.version)
            && existing.manifest.executable.is_file()
        {
            return Ok(existing.clone());
        }
        let now = unix_timestamp();
        let registered_at = registry
            .apps
            .get(&manifest.id)
            .map(|app| app.registered_at)
            .unwrap_or(now);
        let app = RegisteredApp {
            manifest,
            registered_at,
            updated_at: now,
        };
        registry.apps.insert(app.manifest.id.clone(), app.clone());
        self.save_registry(&registry)?;
        Ok(app)
    }

    pub fn unregister(&self, id: &str) -> ServiceResult<RegisteredApp> {
        let _lock = RegistryLock::acquire(&self.root)?;
        let mut registry = self.load_registry()?;
        let app = registry
            .apps
            .remove(id)
            .ok_or_else(|| ServiceError::AppNotFound(id.to_string()))?;
        self.save_registry(&registry)?;
        Ok(app)
    }

    pub fn apps(&self) -> ServiceResult<Vec<RegisteredApp>> {
        let _lock = RegistryLock::acquire(&self.root)?;
        Ok(self.load_registry()?.apps.into_values().collect())
    }

    pub fn app(&self, id: &str) -> ServiceResult<RegisteredApp> {
        let _lock = RegistryLock::acquire(&self.root)?;
        self.load_registry()?
            .apps
            .remove(id)
            .ok_or_else(|| ServiceError::AppNotFound(id.to_string()))
    }

    pub(crate) fn incompatible_apps(&self) -> ServiceResult<Vec<String>> {
        let compatibility = crate::install::installed_system_compatibility();
        Ok(self
            .apps()?
            .into_iter()
            .filter(|app| incompatibility_message(&app.manifest, compatibility).is_some())
            .map(|app| app.manifest.id)
            .collect())
    }

    pub fn runtime(&self) -> ServiceResult<RuntimeInfo> {
        resolve_runtime(&self.runtime).map_err(Into::into)
    }

    pub fn ensure_runtime(&self) -> ServiceResult<RuntimeInfo> {
        ensure_runtime(&self.runtime).map_err(Into::into)
    }

    pub fn ensure_runtime_with_progress(
        &self,
        mut progress: impl FnMut(RuntimeInstallProgress),
    ) -> ServiceResult<RuntimeInfo> {
        crate::updates::retry_quarantined_runtimes()?;
        match resolve_runtime(&self.runtime) {
            Ok(runtime) => Ok(runtime),
            Err(_) => {
                install_user_runtime_with_progress(&self.runtime, &mut progress).map_err(Into::into)
            }
        }
    }

    pub(crate) fn registry_path(&self) -> PathBuf {
        self.root.join("apps.json")
    }

    pub(crate) fn load_registry(&self) -> ServiceResult<RegistryFile> {
        let path = self.registry_path();
        let backup = self.root.join("apps.json.bak");
        let source = if path.is_file() {
            path
        } else if backup.is_file() {
            backup
        } else {
            return Ok(RegistryFile::default());
        };
        let bytes = std::fs::read(&source)?;
        let registry = serde_json::from_slice::<RegistryFile>(&bytes).map_err(|error| {
            ServiceError::Decode {
                path: source,
                source: error,
            }
        })?;
        Ok(registry)
    }

    pub(crate) fn save_registry(&self, registry: &RegistryFile) -> ServiceResult<()> {
        std::fs::create_dir_all(&self.root)?;
        let path = self.registry_path();
        let temporary = self.root.join("apps.json.new");
        let bytes = serde_json::to_vec_pretty(registry).expect("registry is serializable");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        replace_file(&temporary, &path)?;
        Ok(())
    }
}

fn incompatibility_message(
    app: &AppManifest,
    system: crate::SystemCompatibility,
) -> Option<String> {
    let required = app.sabine;
    if required.build == 0 {
        return None;
    }
    if required.major != system.major || required.build < system.minimum_app_build {
        return Some(format!(
            "The developer of {} has not updated it to work with this version of Sabine. The app was not added to Sabine.",
            app.name
        ));
    }
    if required.build > system.build {
        return Some(format!(
            "{} needs Sabine {}, but this computer could not update past Sabine {}.",
            app.name,
            required.label(),
            crate::SabineVersion {
                major: system.major,
                build: system.build,
            }
            .label()
        ));
    }
    None
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct RegistryFile {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) apps: BTreeMap<String, RegisteredApp>,
}

impl Default for RegistryFile {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            apps: BTreeMap::new(),
        }
    }
}

pub(crate) struct RegistryLock {
    _lock: sabine_runtime::FileLock,
}

impl RegistryLock {
    pub(crate) fn acquire(root: &Path) -> ServiceResult<Self> {
        Ok(Self {
            _lock: sabine_runtime::FileLock::acquire(
                &root.join("apps.lock"),
                Duration::from_secs(10),
                |_| {},
            )?,
        })
    }
}

pub(crate) fn replace_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(temporary, destination)?;
    #[cfg(unix)]
    if let Some(parent) = destination.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
