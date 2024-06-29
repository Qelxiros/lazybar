mod battery;
mod clock;
mod cpu;
mod custom;
mod fanotify;
mod inotify;
mod memory;
mod mpd;
mod network;
mod ping;
mod pulseaudio;
mod separator;
mod temp;
mod xwindow;
mod xworkspaces;

pub use battery::Battery;
pub use clock::{precision, Clock};
pub use cpu::Cpu;
pub use custom::Custom;
pub use fanotify::Fanotify;
pub use inotify::Inotify;
pub use memory::Memory;
pub use mpd::Mpd;
pub use network::Network;
pub use ping::Ping;
pub use pulseaudio::Pulseaudio;
pub use separator::Separator;
pub use temp::Temp;
pub use xwindow::XWindow;
pub use xworkspaces::XWorkspaces;

/// Builder structs for panels, courtesy of [`derive_builder`].
pub mod builders {
    pub use super::{
        battery::{BatteryBuilder, BatteryBuilderError},
        clock::{ClockBuilder, ClockBuilderError},
        cpu::{CpuBuilder, CpuBuilderError},
        custom::{CustomBuilder, CustomBuilderError},
        fanotify::{FanotifyBuilder, FanotifyBuilderError},
        inotify::{InotifyBuilder, InotifyBuilderError},
        memory::{MemoryBuilder, MemoryBuilderError},
        mpd::{MpdBuilder, MpdBuilderError},
        network::{NetworkBuilder, NetworkBuilderError},
        ping::{PingBuilder, PingBuilderError},
        pulseaudio::{PulseaudioBuilder, PulseaudioBuilderError},
        separator::{SeparatorBuilder, SeparatorBuilderError},
        temp::{TempBuilder, TempBuilderError},
        xwindow::{XWindowBuilder, XWindowBuilderError},
        xworkspaces::{XWorkspacesBuilder, XWorkspacesBuilderError},
    };
}
