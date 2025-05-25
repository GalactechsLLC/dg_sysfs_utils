use crate::classes::block::BLOCK_CLASS;
use crate::{DEV_DIR, MOUNTS_FILE, SysFsNode, read_value};
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::io::{Error, ErrorKind, SeekFrom};
use std::mem;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader};
use uuid::{Builder, Uuid};

#[derive(Debug, Serialize, Deserialize)]
pub struct SpaceInfo {
    total_space: u64,
    free_space: u64,
    used_space: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum DiskType {
    Mmc,
    Nvme,
    Scsi,
    Test,
    Unknown,
    Virtual,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Partition {
    pub name: String,
    pub uuid: Option<Uuid>,
    pub node: PathBuf,
    pub number: Option<String>,
    pub device: PathBuf,
    pub file_system: Option<FileSystem>,
    pub space_info: Option<SpaceInfo>,
    pub mount_path: Option<PathBuf>,
}

impl SysFsNode for Partition {
    async fn read<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let path = path.as_ref();
        debug!("Parsing Partition at {path:?}");
        match path.file_name() {
            Some(name) => {
                let device = Path::new(DEV_DIR).join(name);
                let name = name.to_string_lossy().to_string();
                let file_system = FileSystem::from_dev(&device).await.unwrap_or_else(|e| {
                    warn!("Failed to open file system at {device:?}: {e:?}");
                    None
                });
                let number = read_value(BLOCK_CLASS, &name, "partition").await.ok();
                let mount_path = get_mount_path(&device).await?;
                let space_info = if let Some(mount_path) = mount_path.as_ref() {
                    get_device_space(mount_path).await?
                } else {
                    let total_space = if let (Some(sectors), Some(sector_size)) = (
                        read_value(BLOCK_CLASS, &name, "size").await.ok(),
                        read_value(BLOCK_CLASS, &name, "queue/hw_sector_size")
                            .await
                            .ok(),
                    ) {
                        let sectors: u64 = sectors.trim().parse().ok().unwrap_or(0);
                        let sector_size: u64 = sector_size.trim().parse().ok().unwrap_or(0);
                        sectors * sector_size
                    } else {
                        0
                    };
                    Some(SpaceInfo {
                        total_space,
                        free_space: 0,
                        used_space: 0,
                    })
                };
                Ok(Partition {
                    name,
                    uuid: None,
                    node: path.to_path_buf(),
                    number,
                    device,
                    file_system,
                    space_info,
                    mount_path,
                })
            }
            None => Err(Error::new(
                ErrorKind::InvalidInput,
                "Failed to read device name from path",
            )),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Disk {
    pub name: String,
    pub node: PathBuf,
    pub disk_type: DiskType,
    pub device: PathBuf,
    pub file_system: Option<FileSystem>,
    pub space_info: Option<SpaceInfo>,
    pub mount_path: Option<PathBuf>,
    pub model: Option<String>,
    pub vendor: Option<String>,
    pub partitions: Vec<Partition>,
}

impl SysFsNode for Disk {
    async fn read<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let path = path.as_ref();
        match path.file_name() {
            Some(name) => {
                debug!("Parsing Disk at {name:?}");
                let model = read_value(BLOCK_CLASS, name, "device/model").await.ok();
                let vendor = read_value(BLOCK_CLASS, name, "device/vendor").await.ok();
                let device = Path::new(DEV_DIR).join(name);
                let name = name.to_string_lossy().to_string();
                let disk_type = if name.starts_with("sd") {
                    DiskType::Scsi
                } else if name.starts_with("nvme") {
                    DiskType::Nvme
                } else {
                    DiskType::Unknown
                };
                let partitions = find_partitions(path).await?;
                let file_system = if partitions.is_empty() {
                    None
                } else {
                    FileSystem::from_dev(&device).await.unwrap_or_else(|e| {
                        warn!("Failed to open file system at {device:?}: {e:?}");
                        None
                    })
                };
                let mount_path = get_mount_path(&device).await?;
                let space_info = if let Some(mount_path) = mount_path.as_ref() {
                    get_device_space(mount_path).await?
                } else {
                    let total_space = if let (Some(sectors), Some(sector_size)) = (
                        read_value(BLOCK_CLASS, &name, "size").await.ok(),
                        read_value(BLOCK_CLASS, &name, "queue/hw_sector_size")
                            .await
                            .ok(),
                    ) {
                        let sectors: u64 = sectors.trim().parse().ok().unwrap_or(0);
                        let sector_size: u64 = sector_size.trim().parse().ok().unwrap_or(0);
                        sectors * sector_size
                    } else {
                        0
                    };
                    Some(SpaceInfo {
                        total_space,
                        free_space: 0,
                        used_space: 0,
                    })
                };
                Ok(Disk {
                    name,
                    node: path.to_path_buf(),
                    disk_type,
                    device,
                    model,
                    vendor,
                    partitions,
                    file_system,
                    mount_path,
                    space_info,
                })
            }
            None => Err(Error::new(
                ErrorKind::InvalidInput,
                "Failed to read device name from path",
            )),
        }
    }
}

pub async fn find_partitions<P: AsRef<Path>>(path: P) -> Result<Vec<Partition>, Error> {
    let mut partitions = vec![];
    let mut entries = tokio::fs::read_dir(path.as_ref()).await?;
    debug!("Searching for Partitions at {:?}", path.as_ref());
    while let Some(entry) = entries.next_entry().await? {
        let file_type = entry.file_type().await?;
        let file_name = entry.file_name();
        if file_type.is_dir() {
            if let Ok(part_num) = read_value(path.as_ref(), file_name, "partition").await {
                let partition_number = usize::from_str(&part_num).unwrap_or_default();
                match Partition::read(&entry.path()).await {
                    Ok(mut partition) => {
                        if partition_number > 0 {
                            partition.uuid =
                                get_part_uuid(path.as_ref(), partition_number).await.ok();
                        }
                        partitions.push(partition)
                    }
                    Err(err) => warn!("Failed to read partition: {:?}", err),
                }
            }
        }
    }
    Ok(partitions)
}

async fn get_mount_path<P: AsRef<Path>>(device: P) -> Result<Option<PathBuf>, Error> {
    // Open /proc/mounts for reading
    let file = tokio::fs::File::open(MOUNTS_FILE).await?;
    let reader = BufReader::new(file);
    // Iterate through each line in /proc/mounts
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        // Split the line by whitespace; fields are:
        // [0]: device, [1]: mount path, [2]: filesystem type, etc.
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && Path::new(parts[0]) == device.as_ref() {
            return Ok(Some(PathBuf::from(parts[1].trim())));
        }
    }
    Ok(None)
}

async fn get_device_space<P: AsRef<Path>>(device: P) -> Result<Option<SpaceInfo>, Error> {
    let device_as_string = device.as_ref().to_string_lossy().to_string();
    let c_mount =
        CString::new(device_as_string).map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
    // Create an uninitialized statvfs structure.
    let mut stat: libc::statvfs = unsafe { mem::zeroed() };
    // Call the statvfs system call on the mount path.
    if unsafe { libc::statvfs(c_mount.as_ptr(), &mut stat) } != 0 {
        return Ok(None);
    }
    // The fragment size is used to scale the block counts.
    let block_size = stat.f_frsize as u64;
    let total_space = stat.f_blocks as u64 * block_size;
    let free_space = stat.f_bfree as u64 * block_size;
    let used_space = total_space - free_space;

    Ok(Some(SpaceInfo {
        total_space,
        free_space,
        used_space,
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub enum FileSystem {
    Btrfs,
    ExFAT,
    Ext2,
    Ext3,
    Ext4,
    F2FS,
    FAT12,
    FAT16,
    FAT32,
    ISO9660,
    JFS,
    NTFS,
    ReiserFS,
    Unknown,
    XFS,
}

impl FileSystem {
    pub async fn from_dev<P: AsRef<Path>>(path: P) -> Result<Option<FileSystem>, Error> {
        debug!("Parsing Filesystem at {:?}", path.as_ref());
        let mut file = tokio::fs::File::open(path).await?;
        // Read the boot sector (first 2048 bytes) for several checks.
        let mut buffer = [0u8; 2048];
        file.seek(SeekFrom::Start(0)).await?;
        file.read_exact(&mut buffer).await?;

        // 1. XFS: "XFSB" at offset 0.
        if &buffer[0..4] == b"XFSB" {
            return Ok(Some(FileSystem::XFS));
        }
        // 2. JFS: "JFS1" at offset 0.
        if &buffer[0..4] == b"JFS1" {
            return Ok(Some(FileSystem::JFS));
        }
        // 3. F2FS: First 4 bytes as little-endian should equal 0xF2F52010.
        let f2fs_magic = u32::from_le_bytes(buffer[..4].try_into().unwrap());
        if f2fs_magic == 0xF2F52010 {
            return Ok(Some(FileSystem::F2FS));
        }
        // 4. NTFS: Check for "NTFS" at offset 3.
        if &buffer[3..7] == b"NTFS" {
            return Ok(Some(FileSystem::NTFS));
        }
        // 5. exFAT: Check for "EXFAT" at offset 3.
        if &buffer[3..8] == b"EXFAT" {
            return Ok(Some(FileSystem::ExFAT));
        }
        // 6. FAT: Check the FS type field.
        // For FAT32, the field at offset 82 (8 bytes) typically starts with "FAT32".
        if buffer[82..90].starts_with(b"FAT32") {
            return Ok(Some(FileSystem::FAT32));
        }
        // For FAT12/16, the field at offset 54 (8 bytes) starts with "FAT12" or "FAT16".
        if buffer[54..62].starts_with(b"FAT12") {
            return Ok(Some(FileSystem::FAT12));
        } else if buffer[54..62].starts_with(b"FAT16") {
            return Ok(Some(FileSystem::FAT16));
        }
        // 7. ext2/3/4: The superblock starts at offset 1024.
        // The magic (0xEF53) is at offset 1024+56 = 1080.
        if buffer[1080] == 0x53 && buffer[1081] == 0xef {
            // Feature flags are stored relative to the superblock start:
            // Compatible features at offset 1024+92 = 1116, and
            // Incompatible features at offset 1024+96 = 1120.
            let feature_compat = u32::from_le_bytes(buffer[1116..1120].try_into().unwrap());
            let feature_incompat = u32::from_le_bytes(buffer[1120..1124].try_into().unwrap());
            let has_journal = (feature_compat & 0x0004) != 0;
            let has_extents = (feature_incompat & 0x40) != 0;
            if !has_journal {
                return Ok(Some(FileSystem::Ext2));
            } else if has_journal && !has_extents {
                return Ok(Some(FileSystem::Ext3));
            } else if has_journal && has_extents {
                return Ok(Some(FileSystem::Ext4));
            }
        }
        // 8. ReiserFS: Magic "ReIsErFs" at offset 256.
        if &buffer[256..264] == b"ReIsErFs" {
            return Ok(Some(FileSystem::ReiserFS));
        }
        // 9. Btrfs: The primary superblock is at offset 65536.
        let mut btrfs_buf = [0u8; 8];
        file.seek(SeekFrom::Start(65536)).await?;
        file.read_exact(&mut btrfs_buf).await?;
        if &btrfs_buf[0..5] == b"BTRFS" {
            return Ok(Some(FileSystem::Btrfs));
        }
        // 10. ISO9660: The primary volume descriptor is at offset 32768.
        let mut iso_buf = [0u8; 2048];
        file.seek(SeekFrom::Start(32768)).await?;
        file.read_exact(&mut iso_buf).await?;
        // For ISO9660, the first byte should be 1 and the next five should be "CD001".
        if iso_buf[0] == 1 && &iso_buf[1..6] == b"CD001" {
            return Ok(Some(FileSystem::ISO9660));
        }
        Ok(None)
    }
}

async fn get_part_uuid<P: AsRef<Path>>(
    disk: P,
    part_index: usize, // 1-based
) -> Result<Uuid, Error> {
    let mut f = File::open(&disk).await?;
    let mut head = [0u8; 0x1C3];
    f.read_exact(&mut head).await?;
    let part_type = head[0x1C2];
    if part_type == 0xEE {
        read_gpt_part_guid(disk, part_index).await
    } else {
        read_mbr_part_guid(disk, part_index).await
    }
}

async fn read_gpt_part_guid<P: AsRef<Path>>(
    disk_path: P,
    part_index: usize,
) -> Result<Uuid, Error> {
    let mut f = File::open(disk_path.as_ref()).await?;
    const SECTOR_SIZE: u64 = 512;
    f.seek(SeekFrom::Start(SECTOR_SIZE)).await?;
    let mut hdr = [0u8; 92];
    f.read_exact(&mut hdr).await?;
    if &hdr[0..8] != b"EFI PART" {
        drop(f);
        return read_mbr_part_guid(disk_path, part_index).await;
    }
    let part_entries_lba = u64::from_le_bytes(hdr[72..80].try_into().unwrap());
    let entry_count = u32::from_le_bytes(hdr[80..84].try_into().unwrap()) as u64;
    let entry_size = u32::from_le_bytes(hdr[84..88].try_into().unwrap()) as u64;
    if part_index == 0 || (part_index as u64) > entry_count {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("Bad Index: {}", part_index),
        ));
    }
    let entry_offset = part_entries_lba * SECTOR_SIZE + (part_index as u64 - 1) * entry_size + 16;
    f.seek(SeekFrom::Start(entry_offset)).await?;
    let mut guid_le = [0u8; 16];
    f.read_exact(&mut guid_le).await?;
    Ok(Uuid::from_bytes_le(guid_le))
}

async fn read_mbr_part_guid<P: AsRef<Path>>(
    disk: P,
    part_index: usize, // e.g. 1 for sda1, 2 for sda2
) -> Result<Uuid, Error> {
    const MBR_SIZE: usize = 512;
    let mut buf = [0u8; MBR_SIZE];
    let mut f = File::open(disk).await?;
    f.read_exact(&mut buf).await?;
    if buf[510] != 0x55 || buf[511] != 0xAA {
        return Err(Error::new(ErrorKind::InvalidData, "Not a Valid Partition"));
    }
    let sig = u32::from_le_bytes([buf[0x1B8], buf[0x1B9], buf[0x1BA], buf[0x1BB]]);
    let mut bytes = [0u8; 16];
    bytes[0..4].copy_from_slice(&sig.to_be_bytes());
    bytes[4..8].copy_from_slice(&sig.to_be_bytes());
    bytes[8..12].copy_from_slice(&(part_index as u32).to_be_bytes());
    bytes[12..16].copy_from_slice(&(part_index as u32).to_be_bytes());
    Ok(Builder::from_random_bytes(bytes).into_uuid())
}
