//! Persistent node identity and bounded libp2p application transport.
pub type Error = Box<dyn std::error::Error + Send + Sync>;
#[cfg(any(feature = "nas", feature = "relay"))]
pub mod transport;
#[cfg(feature = "nas")]
pub fn load_or_create_peer_id(path: &std::path::Path) -> Result<String, Error> {
    Ok(load_or_create_key(path)?.public().to_peer_id().to_string())
}
#[cfg(any(feature = "nas", feature = "relay"))]
pub fn load_or_create_key(path: &std::path::Path) -> Result<libp2p::identity::Keypair, Error> {
    use libp2p::identity::Keypair;
    use std::{
        fs::{self, OpenOptions},
        io::{ErrorKind, Write},
    };
    match fs::read(path) {
        Ok(bytes) => {
            return Ok(Keypair::from_protobuf_encoding(&bytes)?);
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let key = Keypair::generate_ed25519();
    let encoded = key.to_protobuf_encoding()?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(&encoded)?;
            file.sync_all()?;
            // Persist the new directory entry before exposing this identity for registration.
            #[cfg(unix)]
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::File::open(parent)?.sync_all()?;
            }
            Ok(key)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            Ok(Keypair::from_protobuf_encoding(&fs::read(path)?)?)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(all(test, feature = "nas"))]
mod tests {
    use super::*;
    #[test]
    fn identity_survives_restart_and_corruption_is_not_overwritten() {
        let directory = std::env::temp_dir().join(format!(
            "red-leaves-peer-{}",
            crate::identity::secret().unwrap()
        ));
        let path = directory.join("node.key");
        let first = load_or_create_peer_id(&path).unwrap();
        assert!(first.parse::<libp2p::PeerId>().is_ok());
        assert_eq!(first, load_or_create_peer_id(&path).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::write(&path, b"invalid identity").unwrap();
        assert!(load_or_create_peer_id(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid identity");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
