use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::registry::RegistryLock;
use crate::{AppManifest, RegisteredApp, SabineService, ServiceError, ServiceResult};

const INVENTORY: &str = ".sabine-install.json";

#[derive(Deserialize)]
struct Inventory {
    id: String,
    files: BTreeSet<PathBuf>,
}

#[derive(Deserialize, Serialize)]
struct Transaction {
    id: String,
    destination: PathBuf,
    previous_app: Option<RegisteredApp>,
    user_files: Vec<PathBuf>,
    had_destination: bool,
    publishing: bool,
    committed: bool,
}

impl SabineService {
    pub fn install_app_payload(
        &self,
        source: &Path,
        destination: &Path,
        mut manifest: AppManifest,
        mut cancelled: impl FnMut() -> bool,
    ) -> ServiceResult<()> {
        manifest.validate()?;
        let _lock = self.app_update_lock(&manifest.id)?;
        let source = source.canonicalize()?;
        let destination = std::path::absolute(destination)?;
        let parent = destination
            .parent()
            .ok_or_else(|| invalid("installation has no parent"))?;
        fs::create_dir_all(parent)?;
        let destination = parent.canonicalize()?.join(
            destination
                .file_name()
                .ok_or_else(|| invalid("installation has no directory name"))?,
        );
        if source.starts_with(&destination) || destination.starts_with(&source) {
            return Err(invalid("installer source and destination must be separate"));
        }
        if destination.exists() && fs::symlink_metadata(&destination)?.file_type().is_symlink() {
            return Err(invalid(
                "installation destination cannot be a symbolic link",
            ));
        }
        let journal = destination
            .parent()
            .unwrap()
            .join(format!(".{}.sabine-install", manifest.id));
        self.recover_installer(&journal, &manifest.id, &destination)?;
        let inventory = read_inventory(&source, &manifest.id)?;
        let executable_path = manifest.executable.canonicalize()?;
        let executable = executable_path
            .strip_prefix(&source)
            .map_err(|_| invalid("installer executable is outside its payload"))?;
        if !inventory.files.contains(executable) {
            return Err(invalid(
                "installer inventory does not contain its executable",
            ));
        }
        crate::updates::installers::validate_executable(&source.join(executable))?;
        manifest.executable = destination.join(executable);
        let previous_app = match self.app(&manifest.id) {
            Ok(app) => Some(app),
            Err(ServiceError::AppNotFound(_)) => None,
            Err(error) => return Err(error),
        };
        if previous_app.as_ref().is_some_and(|app| {
            crate::types::version_is_newer(&app.manifest.version, &manifest.version)
        }) {
            return Err(invalid(
                "a newer version of this application is already installed",
            ));
        }
        let old = if destination.is_dir() && fs::read_dir(&destination)?.next().is_some() {
            Some(read_inventory(&destination, &manifest.id).map_err(|_| invalid("the destination is not an installation managed by this setup; choose an empty directory or uninstall the previous package first"))?)
        } else {
            None
        };
        fs::create_dir(&journal)?;
        let mut transaction = Transaction {
            id: manifest.id.clone(),
            destination: destination.clone(),
            previous_app,
            user_files: Vec::new(),
            had_destination: destination.exists(),
            publishing: false,
            committed: false,
        };
        if let Err(error) = write_transaction(&journal, &transaction) {
            let _ = fs::remove_dir_all(&journal);
            return Err(error);
        }
        let stage = journal.join("stage");
        let result = (|| {
            fs::create_dir(&stage)?;
            for relative in &inventory.files {
                check_cancelled(&mut cancelled)?;
                copy_file(&source.join(relative), &stage.join(relative))?;
            }
            copy_file(&source.join(INVENTORY), &stage.join(INVENTORY))?;
            if let Some(old) = &old {
                let directories = old
                    .files
                    .iter()
                    .flat_map(|file| file.ancestors().skip(1).map(Path::to_path_buf))
                    .collect();
                collect_user_files(
                    &destination,
                    &destination,
                    &stage,
                    &old.files,
                    &directories,
                    &mut cancelled,
                    &mut transaction.user_files,
                )?;
            }
            check_cancelled(&mut cancelled)?;
            transaction.publishing = true;
            write_transaction(&journal, &transaction)?;
            if transaction.had_destination {
                fs::rename(&destination, journal.join("previous"))
                    .map_err(|error| invalid(&format!("close the installed application and retry; could not move its previous files: {error}")))?;
                sync_directory(destination.parent().unwrap())?;
                for relative in &transaction.user_files {
                    check_cancelled(&mut cancelled)?;
                    fs::rename(
                        journal.join("previous").join(relative),
                        stage.join(relative),
                    )?;
                }
            }
            check_cancelled(&mut cancelled)?;
            fs::rename(&stage, &destination)?;
            sync_directory(destination.parent().unwrap())?;
            self.register(manifest)?;
            transaction.committed = true;
            write_transaction(&journal, &transaction)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.recover_installer(&journal, &transaction.id, &destination)
                .map_err(|recovery| {
                    invalid(&format!(
                        "{error}; rollback failed: {recovery}. Previous files remain in {}",
                        journal.display()
                    ))
                })?;
            return Err(error);
        }
        if let Err(error) = fs::remove_dir_all(&journal) {
            sabine_runtime::report_error(
                "installer",
                format!("installation completed; old payload cleanup will retry: {error}"),
            );
        }
        Ok(())
    }

    fn recover_installer(&self, journal: &Path, id: &str, destination: &Path) -> ServiceResult<()> {
        if !journal.exists() {
            return Ok(());
        }
        if fs::symlink_metadata(journal)?.file_type().is_symlink() {
            return Err(invalid(
                "installation transaction cannot be a symbolic link",
            ));
        }
        if !journal.join("transaction.json").exists() {
            if fs::read_dir(journal)?
                .all(|entry| entry.is_ok_and(|entry| entry.file_name() == "transaction.tmp"))
            {
                fs::remove_dir_all(journal)?;
                return Ok(());
            }
            return Err(invalid("installation transaction has no journal"));
        }
        let transaction: Transaction = read_json(&journal.join("transaction.json"))?;
        if transaction.id != id || transaction.destination != destination {
            return Err(invalid(
                "another installation owns this transaction directory",
            ));
        }
        if transaction.publishing && !transaction.committed {
            let previous = journal.join("previous");
            if previous.exists() {
                for relative in &transaction.user_files {
                    validate_relative_path(relative)?;
                    if fs::symlink_metadata(previous.join(relative)).is_ok() {
                        continue;
                    }
                    let stage = journal.join("stage").join(relative);
                    let source = if fs::symlink_metadata(&stage).is_ok() {
                        stage
                    } else {
                        destination.join(relative)
                    };
                    fs::rename(source, previous.join(relative))?;
                }
            }
            if previous.exists() || !transaction.had_destination {
                if destination.exists() {
                    fs::remove_dir_all(destination)?;
                }
                if previous.exists() {
                    fs::rename(previous, destination)?;
                }
                sync_directory(destination.parent().unwrap())?;
            }
            let _lock = RegistryLock::acquire(&self.root)?;
            let mut registry = self.load_registry()?;
            if let Some(app) = transaction.previous_app {
                registry.apps.insert(id.to_string(), app);
            } else {
                registry.apps.remove(id);
            }
            self.save_registry(&registry)?;
        }
        fs::remove_dir_all(journal)?;
        Ok(())
    }
}

fn read_inventory(root: &Path, id: &str) -> ServiceResult<Inventory> {
    let inventory: Inventory = read_json(&root.join(INVENTORY))?;
    if inventory.id != id || inventory.files.is_empty() || inventory.files.len() > 100_000 {
        return Err(invalid("invalid installer payload identity or inventory"));
    }
    for path in &inventory.files {
        validate_relative_path(path)?;
        let mut ancestor = path.parent();
        while let Some(parent) = ancestor.filter(|path| !path.as_os_str().is_empty()) {
            if fs::symlink_metadata(root.join(parent))
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(invalid("installer inventory traverses a symbolic link"));
            }
            ancestor = parent.parent();
        }
    }
    Ok(inventory)
}

fn collect_user_files(
    root: &Path,
    directory: &Path,
    stage: &Path,
    owned: &BTreeSet<PathBuf>,
    owned_directories: &BTreeSet<PathBuf>,
    cancelled: &mut impl FnMut() -> bool,
    files: &mut Vec<PathBuf>,
) -> ServiceResult<()> {
    for entry in fs::read_dir(directory)? {
        check_cancelled(cancelled)?;
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap();
        if owned.contains(relative) || relative == Path::new(INVENTORY) {
            continue;
        }
        let target = stage.join(relative);
        if entry.file_type()?.is_dir() {
            if !owned_directories.contains(relative) {
                fs::create_dir_all(&target)?;
            }
            collect_user_files(
                root,
                &path,
                stage,
                owned,
                owned_directories,
                cancelled,
                files,
            )?;
        } else {
            if target.exists() {
                return Err(invalid(&format!(
                    "a user file conflicts with the new payload: {}",
                    relative.display()
                )));
            }
            fs::create_dir_all(target.parent().unwrap())?;
            files.push(relative.to_path_buf());
        }
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> ServiceResult<()> {
    if path.as_os_str().is_empty()
        || path == Path::new(INVENTORY)
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        || path.as_os_str().to_string_lossy().contains(':')
    {
        return Err(invalid("installer inventory contains an unsafe path"));
    }
    Ok(())
}

fn sync_directory(path: &Path) -> ServiceResult<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn copy_file(source: &Path, destination: &Path) -> ServiceResult<()> {
    if !fs::symlink_metadata(source)?.file_type().is_file() {
        return Err(invalid("installer payloads require regular files"));
    }
    fs::create_dir_all(destination.parent().unwrap())?;
    fs::copy(source, destination)?;
    #[cfg(windows)]
    {
        let mut permissions = fs::metadata(destination)?.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(destination, permissions)?;
    }
    Ok(())
}

fn check_cancelled(cancelled: &mut impl FnMut() -> bool) -> ServiceResult<()> {
    if cancelled() {
        Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "installation cancelled").into())
    } else {
        Ok(())
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> ServiceResult<T> {
    let file = fs::File::open(path)?;
    use std::io::Read;
    serde_json::from_reader(file.take(16 * 1024 * 1024)).map_err(|source| ServiceError::Decode {
        path: path.into(),
        source,
    })
}

fn write_transaction(journal: &Path, transaction: &Transaction) -> ServiceResult<()> {
    use std::io::Write;
    let temporary = journal.join("transaction.tmp");
    let mut file = fs::File::create(&temporary)?;
    serde_json::to_writer(&mut file, transaction).map_err(|source| ServiceError::Decode {
        path: temporary.clone(),
        source,
    })?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    crate::registry::replace_file(&temporary, &journal.join("transaction.json"))?;
    Ok(())
}

fn invalid(message: &str) -> ServiceError {
    ServiceError::InvalidManifest(message.into())
}
