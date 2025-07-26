use crate::classes::block::BLOCK_CLASS;
use crate::{DEV_DIR, MOUNTS_FILE, SysFsNode, read_value};
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::CString;
use std::io::{Error, ErrorKind, SeekFrom};
use std::mem;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader};
use uuid::{Builder, Uuid};

use crate::STATS_FILE;

fn default_instant() -> Instant {
    Instant::now()
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct DiskStats {
    pub major: u32,
    pub minor: u32,
    pub reads_completed: u64,
    pub reads_merged: u64,
    pub sectors_read: u64,
    pub ms_reading: u64,
    pub writes_completed: u64,
    pub writes_merged: u64,
    pub sectors_written: u64,
    pub ms_writing: u64,
    pub io_in_progress: u64,
    pub ms_io: u64,
    pub weighted_ms_io: u64,
    pub discards_completed: Option<u64>,
    pub discards_merged: Option<u64>,
    pub sectors_discarded: Option<u64>,
    pub ms_discarding: Option<u64>,
    pub flush_requests: Option<u64>,
    pub ms_flushing: Option<u64>,
    #[serde(skip)]
    #[serde(default = "default_instant")]
    pub timestamp: Instant,
}
#[derive(Serialize)]
pub struct DiskUsage {
    pub recently_read: u64,
    pub recently_writen: u64,
    pub total_read: u64,
    pub total_writen: u64,
}
#[derive(Default, Debug)]
pub struct DiskStatsMap {
    current_stats: HashMap<String, DiskStats>,
    previous_stats: HashMap<String, DiskStats>,
}
impl DiskStatsMap {
    pub async fn parse() -> Result<Self, Error> {
        let file = File::open(STATS_FILE).await?;
        let mut lines = BufReader::new(file).lines();
        let mut stat_map = HashMap::<String, DiskStats>::default();
        while let Some(line) = lines.next_line().await? {
            if let Some((name, ds)) = Self::parse_line(&line) {
                stat_map.insert(name, ds);
            }
        }
        Ok(DiskStatsMap {
            current_stats: stat_map,
            previous_stats: HashMap::<String, DiskStats>::default(),
        })
    }
    pub async fn get(&self, name: &str) -> Option<DiskStats> {
        self.current_stats.get(name).copied()
    }
    pub async fn reload(&mut self) -> Result<(), Error> {
        let file = File::open(STATS_FILE).await?;
        let mut lines = BufReader::new(file).lines();
        while let Some(line) = lines.next_line().await? {
            if let Some((name, ds)) = Self::parse_line(&line) {
                if let Some(old_value) = self.current_stats.insert(name.clone(), ds) {
                    self.previous_stats.insert(name, old_value);
                }
            }
        }
        Ok(())
    }
    pub fn get_disk_usage(&self, name: &str) -> Option<DiskUsage> {
        let current_stats = self.current_stats.get(name)?;
        let previous_stats = self.previous_stats.get(name);
        match previous_stats {
            Some(previous_stats) => {
                let seconds_difference = current_stats
                    .timestamp
                    .duration_since(previous_stats.timestamp)
                    .as_millis();
                let delta_read = current_stats
                    .sectors_read
                    .saturating_sub(previous_stats.sectors_read)
                    as u128
                    / seconds_difference;
                let delta_written = current_stats
                    .sectors_written
                    .saturating_sub(previous_stats.sectors_written)
                    as u128
                    / seconds_difference;
                Some(DiskUsage {
                    recently_read: (delta_read / 1000) as u64,
                    recently_writen: (delta_written / 1000) as u64,
                    total_read: current_stats.sectors_read,
                    total_writen: current_stats.sectors_written,
                })
            }
            None => Some(DiskUsage {
                recently_read: 0,
                recently_writen: 0,
                total_read: current_stats.sectors_read,
                total_writen: current_stats.sectors_written,
            }),
        }
    }
    fn parse_line(line: &str) -> Option<(String, DiskStats)> {
        // Split on whitespace
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 14 {
            return None;
        }
        // Parse the first two as u32
        let major = parts[0].parse::<u32>().ok()?;
        let minor = parts[1].parse::<u32>().ok()?;
        let name = parts[2].to_string();

        // Parse the next 11 fields as u64
        let reads_completed = parts[3].parse::<u64>().ok()?;
        let reads_merged = parts[4].parse::<u64>().ok()?;
        let sectors_read = parts[5].parse::<u64>().ok()?;
        let ms_reading = parts[6].parse::<u64>().ok()?;
        let writes_completed = parts[7].parse::<u64>().ok()?;
        let writes_merged = parts[8].parse::<u64>().ok()?;
        let sectors_written = parts[9].parse::<u64>().ok()?;
        let ms_writing = parts[10].parse::<u64>().ok()?;
        let io_in_progress = parts[11].parse::<u64>().ok()?;
        let ms_io = parts[12].parse::<u64>().ok()?;
        let weighted_ms_io = parts[13].parse::<u64>().ok()?;

        //Extended Fields May not Exist
        let (
            discards_completed,
            discards_merged,
            sectors_discarded,
            ms_discarding,
            flush_requests,
            ms_flushing,
        ) = if parts.len() >= 20 {
            (
                parts[14].parse::<u64>().ok(),
                parts[15].parse::<u64>().ok(),
                parts[16].parse::<u64>().ok(),
                parts[17].parse::<u64>().ok(),
                parts[18].parse::<u64>().ok(),
                parts[19].parse::<u64>().ok(),
            )
        } else {
            (None, None, None, None, None, None)
        };
        Some((
            name,
            DiskStats {
                major,
                minor,
                reads_completed,
                reads_merged,
                sectors_read,
                ms_reading,
                writes_completed,
                writes_merged,
                sectors_written,
                ms_writing,
                io_in_progress,
                ms_io,
                weighted_ms_io,
                discards_completed,
                discards_merged,
                sectors_discarded,
                ms_discarding,
                flush_requests,
                ms_flushing,
                timestamp: Instant::now(),
            },
        ))
    }
}
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct SpaceInfo {
    pub total_space: u64,
    pub free_space: u64,
    pub used_space: u64,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum DiskType {
    Emmc,
    Nvme,
    Scsi,
    Test,
    Unknown,
    Virtual,
    Optical,
    Ide,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
                let uuid = if let Some(part_no) = number.as_ref().and_then(|n| n.parse().ok()) {
                    get_part_uuid(path, part_no).await.ok()
                } else {
                    None
                };
                Ok(Partition {
                    name,
                    uuid,
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
                    DiskType::Scsi // Standard SCSI/SATA disks
                } else if name.starts_with("nvme") {
                    DiskType::Nvme // NVMe devices
                } else if name.starts_with("vd") {
                    DiskType::Virtual // Virtio (KVM/QEMU virtual disks)
                } else if name.starts_with("hd") {
                    DiskType::Ide // Legacy IDE disks
                } else if name.starts_with("mmcblk") {
                    DiskType::Emmc // eMMC/SD card storage
                } else if name.starts_with("sr") {
                    DiskType::Optical // CD/DVD drives
                } else {
                    DiskType::Unknown // Fallback
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
    let file = File::open(MOUNTS_FILE).await?;
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

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum FileSystem {
    Btrfs(Uuid),
    ExFAT(Uuid),
    Ext2(Uuid),
    Ext3(Uuid),
    Ext4(Uuid),
    F2FS(Uuid),
    FAT12(Uuid),
    FAT16(Uuid),
    FAT32(Uuid),
    ISO9660, // Usually no UUID
    JFS(Uuid),
    NTFS(Uuid),
    ReiserFS(Uuid),
    Unknown,
    XFS(Uuid),
}

impl FileSystem {
    pub async fn from_dev<P: AsRef<Path>>(path: P) -> Result<Option<FileSystem>, Error> {
        debug!("Parsing Filesystem at {:?}", path.as_ref());
        let mut file = File::open(path.as_ref()).await?;
        // Read the boot sector (first 2048 bytes) for several checks.
        let mut buffer = [0u8; 2048];
        file.seek(SeekFrom::Start(0)).await?;
        file.read_exact(&mut buffer).await?;

        // Detect FS type + attach UUID
        if &buffer[0..4] == b"XFSB" {
            if let Some(uuid) = read_uuid(path.as_ref(), "xfs").await {
                return Ok(Some(FileSystem::XFS(uuid)));
            }
            return Ok(Some(FileSystem::XFS(Uuid::nil())));
        }
        if &buffer[0..4] == b"JFS1" {
            // JFS UUID is at 0xC0 (192) offset in superblock
            let mut f = File::open(path.as_ref()).await?;
            f.seek(SeekFrom::Start(0xC0)).await?;
            let mut uuid_buf = [0u8; 16];
            f.read_exact(&mut uuid_buf).await?;
            return Ok(Some(FileSystem::JFS(Uuid::from_bytes(uuid_buf))));
        }
        let f2fs_magic = u32::from_le_bytes(buffer[..4].try_into().unwrap());
        if f2fs_magic == 0xF2F52010 {
            // F2FS UUID is at 0x08 offset in the superblock (first block)
            let mut f = File::open(path.as_ref()).await?;
            f.seek(SeekFrom::Start(0x08)).await?;
            let mut uuid_buf = [0u8; 16];
            f.read_exact(&mut uuid_buf).await?;
            return Ok(Some(FileSystem::F2FS(Uuid::from_bytes(uuid_buf))));
        }
        if &buffer[3..7] == b"NTFS" {
            if let Some(uuid) = read_uuid(path.as_ref(), "ntfs").await {
                return Ok(Some(FileSystem::NTFS(uuid)));
            }
            return Ok(Some(FileSystem::NTFS(Uuid::nil())));
        }
        if &buffer[3..8] == b"EXFAT" {
            if let Some(uuid) = read_uuid(path.as_ref(), "fat").await {
                return Ok(Some(FileSystem::ExFAT(uuid)));
            }
            return Ok(Some(FileSystem::ExFAT(Uuid::nil())));
        }
        if buffer[82..90].starts_with(b"FAT32") {
            if let Some(uuid) = read_uuid(path.as_ref(), "fat32").await {
                return Ok(Some(FileSystem::FAT32(uuid)));
            }
            return Ok(Some(FileSystem::FAT32(Uuid::nil())));
        }
        if buffer[54..62].starts_with(b"FAT12") {
            if let Some(uuid) = read_uuid(path.as_ref(), "fat").await {
                return Ok(Some(FileSystem::FAT12(uuid)));
            }
            return Ok(Some(FileSystem::FAT12(Uuid::nil())));
        } else if buffer[54..62].starts_with(b"FAT16") {
            if let Some(uuid) = read_uuid(path.as_ref(), "fat").await {
                return Ok(Some(FileSystem::FAT16(uuid)));
            }
            return Ok(Some(FileSystem::FAT16(Uuid::nil())));
        }
        if buffer[1080] == 0x53 && buffer[1081] == 0xef {
            let feature_compat = u32::from_le_bytes(buffer[1116..1120].try_into().unwrap());
            let feature_incompat = u32::from_le_bytes(buffer[1120..1124].try_into().unwrap());
            let has_journal = (feature_compat & 0x0004) != 0;
            let has_extents = (feature_incompat & 0x40) != 0;
            let uuid = read_uuid(path.as_ref(), "ext").await.unwrap_or(Uuid::nil());
            if !has_journal {
                return Ok(Some(FileSystem::Ext2(uuid)));
            } else if has_journal && !has_extents {
                return Ok(Some(FileSystem::Ext3(uuid)));
            } else if has_journal && has_extents {
                return Ok(Some(FileSystem::Ext4(uuid)));
            }
        }
        if &buffer[256..264] == b"ReIsErFs" {
            // ReiserFS UUID at 0x34 (52) offset in superblock
            let mut f = tokio::fs::File::open(path.as_ref()).await?;
            f.seek(SeekFrom::Start(0x34)).await?;
            let mut uuid_buf = [0u8; 16];
            f.read_exact(&mut uuid_buf).await?;
            return Ok(Some(FileSystem::ReiserFS(Uuid::from_bytes(uuid_buf))));
        }
        let mut btrfs_buf = [0u8; 8];
        file.seek(SeekFrom::Start(65536)).await?;
        file.read_exact(&mut btrfs_buf).await?;
        if &btrfs_buf[0..5] == b"BTRFS" {
            if let Some(uuid) = read_uuid(path.as_ref(), "btrfs").await {
                return Ok(Some(FileSystem::Btrfs(uuid)));
            }
            return Ok(Some(FileSystem::Btrfs(Uuid::nil())));
        }
        let mut iso_buf = [0u8; 2048];
        file.seek(SeekFrom::Start(32768)).await?;
        file.read_exact(&mut iso_buf).await?;
        if iso_buf[0] == 1 && &iso_buf[1..6] == b"CD001" {
            return Ok(Some(FileSystem::ISO9660));
        }

        Ok(None)
    }
}

// Helper to wrap reading a UUID for a given FS type
async fn read_uuid(dev: &Path, fs: &str) -> Option<Uuid> {
    use tokio::io::AsyncSeekExt;
    let mut f = File::open(dev).await.ok()?;
    let mut buf = [0u8; 512];
    match fs {
        "ext" => {
            let mut sb = [0u8; 1024 + 0x78];
            f.read_exact(&mut sb).await.ok()?;
            Some(Uuid::from_bytes(
                sb[1024 + 0x68..1024 + 0x78].try_into().ok()?,
            ))
        }
        "xfs" => {
            f.read_exact(&mut buf[..64]).await.ok()?;
            Some(Uuid::from_bytes(buf[32..48].try_into().ok()?))
        }
        "btrfs" => {
            f.seek(SeekFrom::Start(65536 + 0x20)).await.ok()?;
            f.read_exact(&mut buf[..16]).await.ok()?;
            Some(Uuid::from_bytes(buf[..16].try_into().ok()?))
        }
        "fat" => {
            f.read_exact(&mut buf).await.ok()?;
            let offset = 0x43; // FAT12/16
            let mut id = [0u8; 16];
            id[..4].copy_from_slice(&buf[offset..offset + 4]);
            Some(Uuid::from_bytes(id))
        }
        "fat32" => {
            f.read_exact(&mut buf).await.ok()?;
            let mut id = [0u8; 16];
            id[..4].copy_from_slice(&buf[0x67..0x6B]);
            Some(Uuid::from_bytes(id))
        }
        "ntfs" => {
            f.seek(SeekFrom::Start(0x48)).await.ok()?; // NTFS Volume Serial
            f.read_exact(&mut buf[..8]).await.ok()?;
            let mut id = [0u8; 16];
            id[..8].copy_from_slice(&buf[..8]);
            Some(Uuid::from_bytes(id))
        }
        _ => None,
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
            format!("Bad Index: {part_index}"),
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
