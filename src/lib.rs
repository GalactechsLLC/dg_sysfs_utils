use log::debug;
use std::io::{Error, ErrorKind};
use std::path::Path;

pub mod classes;

const SYSFS_DIR: &str = "/sys/class";
const DEV_DIR: &str = "/dev";
const MOUNTS_FILE: &str = "/proc/mounts";
const STATS_FILE: &str = "/proc/diskstats";

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classes::block::{BlockDevice, BlockEnumerator};
    use tokio::fs;

    #[tokio::test]
    pub async fn test_block_enumerator() {
        use dg_logger::DruidGardenLogger;
        use log::{Level, info};

        // Initialize logger for visible output
        let _logger = DruidGardenLogger::build()
            .current_level(Level::Info)
            .init()
            .unwrap();

        let enumerator = BlockEnumerator::new();
        let devices = enumerator.get_devices().await.unwrap();

        info!("--- Block Devices Detected ---");
        for dev in devices {
            match dev {
                BlockDevice::Disk(d) => {
                    info!("Disk: {}", d.name);
                    if let Some(model) = d.model.as_ref() {
                        info!("  Model: {}", model);
                    }
                    if let Some(vendor) = d.vendor.as_ref() {
                        info!("  Vendor: {}", vendor);
                    }
                    info!("  Type: {:?}", d.disk_type);
                    if let Some(fs) = d.file_system.as_ref() {
                        info!("  Filesystem: {:?}", fs);
                    }
                    if let Some(mount) = d.mount_path.as_ref() {
                        info!("  Mounted at: {}", mount.display());
                    }

                    if !d.partitions.is_empty() {
                        info!("  Partitions:");
                        for p in &d.partitions {
                            info!("    {}:", p.name);
                            if let Some(fs) = p.file_system.as_ref() {
                                info!("      FS: {:?}", fs);
                            }
                            if let Some(uuid) = p.uuid.as_ref() {
                                info!("      PARTUUID: {}", uuid);
                            }
                            if let Some(mount) = p.mount_path.as_ref() {
                                info!("      Mounted at: {}", mount.display());
                            }
                        }
                    }
                }
                BlockDevice::Partition(p) => {
                    info!("Partition: {}", p.name);
                    if let Some(fs) = p.file_system.as_ref() {
                        info!("  FS: {:?}", fs);
                    }
                    if let Some(uuid) = p.uuid.as_ref() {
                        info!("  PARTUUID: {}", uuid);
                    }
                    if let Some(mount) = p.mount_path.as_ref() {
                        info!("  Mounted at: {}", mount.display());
                    }
                }
                BlockDevice::Loopback => info!("Loopback device"),
                BlockDevice::Ram => info!("RAM disk"),
                BlockDevice::ZRam => info!("ZRAM device"),
                BlockDevice::DeviceMapper => info!("Device Mapper volume"),
            }
        }
    }
}
