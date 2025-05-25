pub mod disk;

use crate::classes::block::disk::{Disk, Partition};
use crate::{SYSFS_DIR, SysFsNode, read_value};
use serde::{Deserialize, Serialize};
use std::io::{Error, ErrorKind};
use std::path::Path;

const BLOCK_CLASS: &str = "block";

#[derive(Debug, Serialize, Deserialize)]
pub enum BlockDevice {
    Disk(Disk),
    Partition(Partition),
    Loopback,
    Ram,
    ZRam,
    DeviceMapper,
}
impl SysFsNode for BlockDevice {
    async fn read<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let path = path.as_ref();
        match path.file_name() {
            Some(name) => {
                let name = name.to_string_lossy().to_string();
                Ok(if name.starts_with("loop") {
                    BlockDevice::Loopback
                } else if name.starts_with("ram") {
                    BlockDevice::Ram
                } else if name.starts_with("zram") {
                    BlockDevice::ZRam
                } else if name.starts_with("dm") {
                    BlockDevice::DeviceMapper
                } else {
                    match read_value(BLOCK_CLASS, &name, "partition").await.ok() {
                        Some(_) => BlockDevice::Partition(Partition::read(path).await?),
                        None => BlockDevice::Disk(Disk::read(path).await?),
                    }
                })
            }
            None => Err(Error::new(
                ErrorKind::InvalidInput,
                "Failed to read device name from path",
            )),
        }
    }
}

#[derive(Debug, Default)]
pub struct BlockEnumerator {}
impl BlockEnumerator {
    pub fn new() -> BlockEnumerator {
        BlockEnumerator {}
    }
    pub async fn get_devices(&self) -> Result<Vec<BlockDevice>, Error> {
        let path = Path::new(SYSFS_DIR).join(BLOCK_CLASS);
        let mut block_devices = vec![];
        let mut entries = tokio::fs::read_dir(path).await?;
        while let Some(entry) = entries.next_entry().await? {
            block_devices.push(BlockDevice::read(entry.path()).await?);
        }
        Ok(block_devices)
    }
}

#[tokio::test]
pub async fn test_block_enumerator() {
    use log::LevelFilter;
    use simple_logger::SimpleLogger;
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .init()
        .unwrap();
    let devices = BlockEnumerator::new().get_devices().await.unwrap();
    for device in devices {
        println!("{device:?}")
    }
}
