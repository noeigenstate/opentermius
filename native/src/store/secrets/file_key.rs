//! Linux fallback: a user-owned 0600 master key, with atomic add-only creation.
//! This protects against other users, not someone who can read this user's files.
use super::*;
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};

pub(super) fn load(directory: &Path, create: bool) -> Result<Zeroizing<[u8; 32]>> {
    if create {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    let metadata = fs::metadata(directory).context("凭据目录不存在")?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o022 == 0,
        "凭据目录必须属于当前用户且不可由其他用户写入"
    );
    let path = directory.join("master-key-v1");
    match read(&path) {
        Ok(key) => return Ok(key),
        Err(error)
            if create
                && error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
        Err(error) => return Err(error).context("无法读取本地主密钥"),
    }
    let key = random_key()?;
    let temporary = directory.join(format!(".master-key-{}", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(key.as_ref())?;
        file.sync_all()?;
        // A competing process may win creation; always read the winning key.
        match fs::hard_link(&temporary, &path) {
            Ok(()) => fs::File::open(directory)?.sync_all()?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result.context("无法保存本地主密钥")?;
    read(&path)
}

fn read(path: &Path) -> Result<Zeroizing<[u8; 32]>> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o777 == 0o600
            && metadata.len() == 32,
        "本地主密钥必须是当前用户拥有的 0600 文件（32 字节）"
    );
    let mut key = Zeroizing::new([0; 32]);
    file.read_exact(key.as_mut())?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn file_key_survives_reload_and_missing_key_is_not_recreated_on_decrypt() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("config");
        assert!(load(&dir, false).is_err());
        assert!(!dir.exists());
        let key = load(&dir, true).unwrap();
        let encrypted = encrypt(&key, "test secret").unwrap();
        assert_eq!(
            decrypt(&load(&dir, false).unwrap(), &encrypted).unwrap(),
            "test secret"
        );
        assert_eq!(
            fs::metadata(dir.join("master-key-v1"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_file(dir.join("master-key-v1")).unwrap();
        assert!(load(&dir, false).is_err());
        assert!(!dir.join("master-key-v1").exists());
    }

    #[test]
    fn file_key_creation_is_safe_across_threads() {
        let temp = tempfile::tempdir().unwrap();
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let dir = temp.path().to_owned();
                std::thread::spawn(move || load(&dir, true).unwrap())
            })
            .collect();
        let keys: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert!(keys.iter().all(|k| k[..] == keys[0][..]));
    }

    #[test]
    fn file_key_rejects_insecure_permissions_corruption_and_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        load(temp.path(), true).unwrap();
        let path = temp.path().join("master-key-v1");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load(temp.path(), true).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, b"short").unwrap();
        assert!(load(temp.path(), true).is_err());
        fs::remove_file(&path).unwrap();
        let other = temp.path().join("other");
        fs::write(&other, [0; 32]).unwrap();
        symlink(&other, &path).unwrap();
        assert!(load(temp.path(), true).is_err());
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o777)).unwrap();
        assert!(load(temp.path(), true).is_err());
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
}
