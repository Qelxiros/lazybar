#[cfg(feature = "battery")]
mod battery;
#[cfg(feature = "clock")]
mod clock;
#[cfg(feature = "cpu")]
mod cpu;
#[cfg(feature = "custom")]
mod custom;
#[cfg(feature = "github")]
mod github;
#[cfg(feature = "inotify")]
mod inotify;
#[cfg(feature = "memory")]
mod memory;
#[cfg(feature = "mpd")]
mod mpd;
#[cfg(feature = "network")]
mod network;
#[cfg(feature = "ping")]
mod ping;
#[cfg(feature = "pulseaudio")]
mod pulseaudio;
#[cfg(feature = "separator")]
mod separator;
#[cfg(feature = "storage")]
mod storage;
#[cfg(feature = "systray")]
mod systray;
#[cfg(feature = "temp")]
mod temp;
#[cfg(feature = "xwindow")]
mod xwindow;
#[cfg(feature = "xworkspaces")]
mod xworkspaces;

#[cfg(feature = "battery")]
pub use battery::Battery;
#[cfg(feature = "clock")]
pub use clock::Clock;
#[cfg(feature = "cpu")]
pub use cpu::Cpu;
#[cfg(feature = "custom")]
pub use custom::Custom;
#[cfg(feature = "github")]
pub use github::Github;
#[cfg(feature = "inotify")]
pub use inotify::Inotify;
#[cfg(feature = "memory")]
pub use memory::Memory;
#[cfg(feature = "mpd")]
pub use mpd::Mpd;
#[cfg(feature = "network")]
pub use network::Network;
#[cfg(feature = "ping")]
pub use ping::Ping;
#[cfg(feature = "pulseaudio")]
pub use pulseaudio::Pulseaudio;
#[cfg(feature = "separator")]
pub use separator::Separator;
#[cfg(feature = "storage")]
pub use storage::Storage;
#[cfg(feature = "systray")]
pub use systray::Systray;
#[cfg(feature = "temp")]
pub use temp::Temp;
#[cfg(feature = "xwindow")]
pub use xwindow::XWindow;
#[cfg(feature = "xworkspaces")]
pub use xworkspaces::XWorkspaces;

/// Builder structs for panels, courtesy of [`derive_builder`].
pub mod builders {
    #[cfg(feature = "battery")]
    pub use super::battery::{BatteryBuilder, BatteryBuilderError};
    #[cfg(feature = "clock")]
    pub use super::clock::{ClockBuilder, ClockBuilderError};
    #[cfg(feature = "cpu")]
    pub use super::cpu::{CpuBuilder, CpuBuilderError};
    #[cfg(feature = "custom")]
    pub use super::custom::{CustomBuilder, CustomBuilderError};
    #[cfg(feature = "github")]
    pub use super::github::{GithubBuilder, GithubBuilderError};
    #[cfg(feature = "inotify")]
    pub use super::inotify::{InotifyBuilder, InotifyBuilderError};
    #[cfg(feature = "memory")]
    pub use super::memory::{MemoryBuilder, MemoryBuilderError};
    #[cfg(feature = "mpd")]
    pub use super::mpd::{MpdBuilder, MpdBuilderError};
    #[cfg(feature = "network")]
    pub use super::network::{NetworkBuilder, NetworkBuilderError};
    #[cfg(feature = "ping")]
    pub use super::ping::{PingBuilder, PingBuilderError};
    #[cfg(feature = "pulseaudio")]
    pub use super::pulseaudio::{PulseaudioBuilder, PulseaudioBuilderError};
    #[cfg(feature = "separator")]
    pub use super::separator::{SeparatorBuilder, SeparatorBuilderError};
    #[cfg(feature = "storage")]
    pub use super::storage::{StorageBuilder, StorageBuilderError};
    #[cfg(feature = "systray")]
    pub use super::systray::{SystrayBuilder, SystrayBuilderError};
    #[cfg(feature = "temp")]
    pub use super::temp::{TempBuilder, TempBuilderError};
    #[cfg(feature = "xwindow")]
    pub use super::xwindow::{XWindowBuilder, XWindowBuilderError};
    #[cfg(feature = "xworkspaces")]
    pub use super::xworkspaces::{XWorkspacesBuilder, XWorkspacesBuilderError};
}
