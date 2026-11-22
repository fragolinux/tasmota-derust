//! Views (tabs) and the columns they can show.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum View {
    /// Big buttons and the main values of every device (no table).
    Dashboard,
    Home,
    Health,
    Firmware,
    Wifi,
    Mqtt,
}

impl View {
    pub const ALL: [View; 6] = [
        View::Dashboard,
        View::Home,
        View::Health,
        View::Firmware,
        View::Wifi,
        View::Mqtt,
    ];

    pub fn label(self) -> &'static str {
        match self {
            View::Dashboard => "Dashboard",
            View::Home => "Home",
            View::Health => "Health",
            View::Firmware => "Firmware",
            View::Wifi => "Wifi",
            View::Mqtt => "MQTT",
        }
    }

    pub fn default_columns(self) -> &'static [Col] {
        use Col::*;
        match self {
            View::Dashboard => &[],
            View::Home => &[Power, Telemetry, Module, Uptime, Rssi],
            View::Health => &[
                Uptime,
                BootCount,
                RestartReason,
                LoadAvg,
                Sleep,
                MqttCount,
                LinkCount,
                Downtime,
                Rssi,
            ],
            View::Firmware => &[Version, Update, Core, Sdk, ProgramSize, Free, OtaUrl],
            View::Wifi => &[
                Hostname, Mac, Ip, Gateway, Ssid, Ap, Bssid, Channel, Rssi, LinkCount, Downtime,
            ],
            View::Mqtt => &[
                Topic,
                FullTopic,
                CommandTopic,
                StatTopic,
                TeleTopic,
                FallbackTopic,
                GroupTopic,
            ],
        }
    }
}

/// Every column except "Device", which is always shown first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Col {
    Power,
    Telemetry,
    Module,
    Uptime,
    LoadAvg,
    LinkCount,
    BootCount,
    RestartReason,
    Sleep,
    MqttCount,
    Downtime,
    Rssi,
    Version,
    Core,
    Sdk,
    ProgramSize,
    Free,
    OtaUrl,
    Update,
    Hostname,
    Mac,
    Ip,
    Gateway,
    Ssid,
    Ap,
    Bssid,
    Channel,
    Topic,
    FullTopic,
    CommandTopic,
    StatTopic,
    TeleTopic,
    FallbackTopic,
    GroupTopic,
}

impl Col {
    pub const ALL: [Col; 34] = [
        Col::Power,
        Col::Telemetry,
        Col::Module,
        Col::Uptime,
        Col::LoadAvg,
        Col::LinkCount,
        Col::BootCount,
        Col::RestartReason,
        Col::Sleep,
        Col::MqttCount,
        Col::Downtime,
        Col::Rssi,
        Col::Version,
        Col::Core,
        Col::Sdk,
        Col::ProgramSize,
        Col::Free,
        Col::OtaUrl,
        Col::Update,
        Col::Hostname,
        Col::Mac,
        Col::Ip,
        Col::Gateway,
        Col::Ssid,
        Col::Ap,
        Col::Bssid,
        Col::Channel,
        Col::Topic,
        Col::FullTopic,
        Col::CommandTopic,
        Col::StatTopic,
        Col::TeleTopic,
        Col::FallbackTopic,
        Col::GroupTopic,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Col::Power => "Power",
            Col::Telemetry => "Telemetry",
            Col::Module => "Module",
            Col::Uptime => "Uptime",
            Col::LoadAvg => "LoadAvg",
            Col::LinkCount => "LinkCount",
            Col::BootCount => "BootCount",
            Col::RestartReason => "RestartReason",
            Col::Sleep => "Sleep",
            Col::MqttCount => "MqttCount",
            Col::Downtime => "Downtime",
            Col::Rssi => "RSSI",
            Col::Version => "Version",
            Col::Core => "Core",
            Col::Sdk => "SDK",
            Col::ProgramSize => "ProgramSize",
            Col::Free => "Free",
            Col::OtaUrl => "OtaUrl",
            Col::Update => "Update",
            Col::Hostname => "Hostname",
            Col::Mac => "Mac",
            Col::Ip => "IPAddress",
            Col::Gateway => "Gateway",
            Col::Ssid => "SSId",
            Col::Ap => "AP",
            Col::Bssid => "BSSId",
            Col::Channel => "Channel",
            Col::Topic => "Topic",
            Col::FullTopic => "FullTopic",
            Col::CommandTopic => "CommandTopic",
            Col::StatTopic => "StatTopic",
            Col::TeleTopic => "TeleTopic",
            Col::FallbackTopic => "FallbackTopic",
            Col::GroupTopic => "GroupTopic",
        }
    }

    pub fn width(self) -> f32 {
        match self {
            Col::Power => 110.0,
            Col::Telemetry => 130.0,
            Col::Module => 140.0,
            Col::RestartReason => 170.0,
            Col::Version | Col::Sdk => 150.0,
            Col::OtaUrl => 380.0,
            Col::Update => 220.0,
            Col::Hostname => 210.0,
            Col::Mac | Col::Bssid => 150.0,
            Col::Ssid => 160.0,
            Col::Topic | Col::FullTopic | Col::GroupTopic => 160.0,
            Col::CommandTopic | Col::StatTopic | Col::TeleTopic | Col::FallbackTopic => 220.0,
            Col::Uptime | Col::Ip | Col::Gateway => 110.0,
            Col::Rssi | Col::Ap => 60.0,
            _ => 90.0,
        }
    }

    /// Right-align numeric columns.
    pub fn numeric(self) -> bool {
        matches!(
            self,
            Col::Uptime
                | Col::LoadAvg
                | Col::LinkCount
                | Col::BootCount
                | Col::Sleep
                | Col::MqttCount
                | Col::Downtime
                | Col::ProgramSize
                | Col::Free
                | Col::Channel
        )
    }
}
