use crate::{SYSFS_DIR, SysFsNode, read_value, value_dir_exists};
use log::error;
use std::fmt::Display;
use std::io::{Error, ErrorKind};
use std::path::Path;

const NET_CLASS: &str = "net";

#[derive(Debug)]
pub struct DeviceSettings {
    pub name: String,
    pub mac_address: String,
    pub interface_index: String,
    pub mtu: String,
    pub state: String,
    pub duplex: String,
    pub speed: String,
    pub carrier: String,
    pub carrier_changes: String,
    pub wireless: bool,
    pub flags: String,
}

pub enum NetType {
    Unknown = 0,
    Ethernet = 1,
    ExEthernet = 2,
    AmateurRadio = 3,
    ProNet = 4,
    Chaos = 5,
    IEEE802 = 6,
    ARCnet = 7,
    AppleTalk = 8,
    DataLinkConnectionIdentifier = 15,
    AsyncTransferMode = 19,
    MetricomStrip = 23,
    IEEE1394 = 24, //Firewire
    EUI64 = 27,
    InfiniBand = 32,
    Loopback = 772,
}
impl From<&str> for NetType {
    fn from(s: &str) -> Self {
        match s {
            "1" => NetType::Ethernet,
            "2" => NetType::ExEthernet,
            "3" => NetType::AmateurRadio,
            "4" => NetType::ProNet,
            "5" => NetType::Chaos,
            "6" => NetType::IEEE802,
            "7" => NetType::ARCnet,
            "8" => NetType::AppleTalk,
            "15" => NetType::DataLinkConnectionIdentifier,
            "19" => NetType::AsyncTransferMode,
            "23" => NetType::MetricomStrip,
            "24" => NetType::IEEE1394,
            "27" => NetType::EUI64,
            "32" => NetType::InfiniBand,
            "772" => NetType::Loopback,
            _ => NetType::Unknown,
        }
    }
}
impl Display for NetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetType::Unknown => write!(f, "Unknown"),
            NetType::Ethernet => write!(f, "Ethernet"),
            NetType::ExEthernet => write!(f, "ExEthernet"),
            NetType::AmateurRadio => write!(f, "AmateurRadio"),
            NetType::ProNet => write!(f, "ProNet"),
            NetType::Chaos => write!(f, "Chaos"),
            NetType::IEEE802 => write!(f, "IEEE802"),
            NetType::ARCnet => write!(f, "ARCnet"),
            NetType::AppleTalk => write!(f, "AppleTalk"),
            NetType::DataLinkConnectionIdentifier => write!(f, "DataLinkConnectionIdentifier"),
            NetType::AsyncTransferMode => write!(f, "AsyncTransferMode"),
            NetType::MetricomStrip => write!(f, "MetricomStrip"),
            NetType::IEEE1394 => write!(f, "IEEE1394"),
            NetType::EUI64 => write!(f, "EUI64"),
            NetType::InfiniBand => write!(f, "InfiniBand"),
            NetType::Loopback => write!(f, "Loopback"),
        }
    }
}

#[derive(Debug)]
pub enum NetDevice {
    Physical(Box<DeviceSettings>),
    Loopback,
    Invalid(String),
}
impl SysFsNode for NetDevice {
    async fn read<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let path = path.as_ref();
        match path.file_name() {
            Some(name) => {
                let name = name.to_string_lossy().to_string();
                let wireless = value_dir_exists(NET_CLASS, &name, "wireless").await;
                let mac_address = read_value(NET_CLASS, &name, "address").await?;
                let interface_index = read_value(NET_CLASS, &name, "ifindex").await?;
                let mtu = read_value(NET_CLASS, &name, "mtu").await?;
                let state = read_value(NET_CLASS, &name, "operstate").await?;
                let net_type = read_value(NET_CLASS, &name, "type").await?;
                let duplex = read_value(NET_CLASS, &name, "duplex")
                    .await
                    .unwrap_or(String::from("unknown"));
                let speed = read_value(NET_CLASS, &name, "speed")
                    .await
                    .unwrap_or(String::from("unknown"));
                let carrier = read_value(NET_CLASS, &name, "carrier").await?;
                let carrier_changes = read_value(NET_CLASS, &name, "carrier_changes").await?;
                let flags = read_value(NET_CLASS, &name, "flags").await?;
                match NetType::from(net_type.as_str()) {
                    NetType::Ethernet => Ok(NetDevice::Physical(Box::new(DeviceSettings {
                        name,
                        wireless,
                        mac_address,
                        interface_index,
                        mtu,
                        state,
                        duplex,
                        speed,
                        carrier,
                        carrier_changes,
                        flags,
                    }))),
                    NetType::Loopback => Ok(NetDevice::Loopback),
                    _ => Err(Error::new(
                        ErrorKind::InvalidInput,
                        format!("Invalid Net Device Type: {net_type}"),
                    )),
                }
            }
            None => Err(Error::new(
                ErrorKind::InvalidInput,
                "Failed to read device name from path",
            )),
        }
    }
}

#[derive(Debug, Default)]
pub struct NetEnumerator {}
impl NetEnumerator {
    pub fn new() -> NetEnumerator {
        NetEnumerator {}
    }
    pub async fn get_devices(&self) -> Result<Vec<NetDevice>, Error> {
        let path = Path::new(SYSFS_DIR).join(NET_CLASS);
        let mut net_devices = vec![];
        let mut entries = tokio::fs::read_dir(&path).await?;
        while let Some(entry) = entries.next_entry().await? {
            net_devices.push(NetDevice::read(entry.path()).await.unwrap_or_else(|e| {
                let err = format!("Failed to Parse Netdevice at {:?} - {e:?}", entry.path());
                error!("{}", err);
                NetDevice::Invalid(err)
            }))
        }
        Ok(net_devices)
    }
}

#[tokio::test]
pub async fn test_net_enumerator() {
    use dg_logger::DruidGardenLogger;
    use log::Level;
    use log::info;
    let _logger = DruidGardenLogger::build()
        .current_level(Level::Info)
        .init()
        .unwrap();
    let devices = NetEnumerator::new().get_devices().await.unwrap();
    for device in devices {
        info!("{device:?}")
    }
}
