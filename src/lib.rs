use log::debug;
use std::io::{Error, ErrorKind};
use std::path::Path;

pub mod classes;

const SYSFS_DIR: &str = "/sys/class";
const DEV_DIR: &str = "/dev";
const MOUNTS_FILE: &str = "/proc/mounts";

#[allow(async_fn_in_trait)]
pub trait SysFsNode: Sized {
    async fn read<P: AsRef<Path>>(path: P) -> Result<Self, Error>;
}
pub async fn read_value<C, N, K>(class: C, node: N, key: K) -> Result<String, Error>
where
    C: AsRef<Path>,
    N: AsRef<Path>,
    K: AsRef<Path>,
{
    let path = Path::new(SYSFS_DIR)
        .join(class.as_ref())
        .join(node.as_ref())
        .join(key.as_ref());
    if path.is_file() {
        tokio::fs::read_to_string(&path)
            .await
            .map(|s| s.trim().to_string())
            .map_err(|e| {
                let err = format!("Failed to Parse Value at {path:?} - {e:?}",);
                debug!("{}", err);
                e
            })
    } else {
        Err(Error::new(
            ErrorKind::IsADirectory,
            format!("{}", path.display()),
        ))
    }
}
pub async fn value_dir_exists<C, N, K>(class: C, node: N, key: K) -> bool
where
    C: AsRef<Path>,
    N: AsRef<Path>,
    K: AsRef<Path>,
{
    let path = Path::new(SYSFS_DIR)
        .join(class.as_ref())
        .join(node.as_ref())
        .join(key.as_ref());
    path.exists() && path.is_dir()
}
