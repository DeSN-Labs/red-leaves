use crate::identity::Error;
use red_leaves_protocol::encryption;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};
use zeroize::Zeroizing;

pub struct KeyStore {
    directory: PathBuf,
    master_path: PathBuf,
    lock: Mutex<()>,
}
impl KeyStore {
    pub fn new(directory: PathBuf, master_path: PathBuf) -> Self {
        Self {
            directory,
            master_path,
            lock: Mutex::new(()),
        }
    }
    fn master(&self) -> Result<Zeroizing<[u8; 32]>, Error> {
        if !self.master_path.exists()
            && self.directory.exists()
            && std::fs::read_dir(&self.directory)?.next().is_some()
        {
            return Err(
                "NAS master key is missing. Restore it from backup before using existing user keys"
                    .into(),
            );
        }
        let candidate = encryption::random_key()?;
        let stored = secret_file(&self.master_path, &*candidate)?;
        Ok(Zeroizing::new(
            stored
                .as_slice()
                .try_into()
                .map_err(|_| "Invalid NAS master key")?,
        ))
    }
    pub fn pair(&self, owner: &str) -> Result<(Zeroizing<Vec<u8>>, Vec<u8>), Error> {
        let address = crate::identity::address(owner)?;
        let _guard = self.lock.lock().map_err(|_| "Key store unavailable")?;
        let master = self.master()?;
        let path = self.directory.join(format!("{}.key", hex::encode(address)));
        let aad = format!("red-leaves/nas-user-key/1/{owner}");
        let encoded = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let (secret, _) = encryption::key_pair()?;
                let sealed = encryption::seal(&master, &secret, aad.as_bytes())?;
                secret_file(&path, &serde_json::to_vec(&sealed)?)?.to_vec()
            }
            Err(error) => return Err(error.into()),
        };
        if encoded.len() > 4096 {
            return Err("Invalid NAS key file".into());
        }
        let sealed = serde_json::from_slice(&encoded)?;
        let secret = encryption::open(&master, &sealed, aad.as_bytes())?;
        let public = encryption::public_key(&secret)?;
        Ok((secret, public))
    }
}

fn secret_file(path: &Path, candidate: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(candidate)?;
            file.sync_all()?;
            #[cfg(unix)]
            if let Some(parent) = path.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return Err("Invalid NAS key file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("NAS key file permissions must be 600".into());
        }
    }
    Ok(Zeroizing::new(fs::read(path)?))
}
