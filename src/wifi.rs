use crate::{latency, network::IP};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
    ptr::null_mut,
};
use windows_sys::{
    Win32::{
        Foundation::LocalFree,
        NetworkManagement::IpHelper::{GetIfEntry, MIB_IFROW},
        System::Power::{
            GetSystemPowerStatus, PowerGetActiveScheme, PowerReadACValueIndex,
            PowerSetActiveScheme, PowerWriteACValueIndex, SYSTEM_POWER_STATUS,
        },
    },
    core::GUID,
};

const WIRELESS: GUID = GUID::from_u128(0x19cbb8fa_5279_450e_9fac_8a3d5fedd0c1);
const SAVING: GUID = GUID::from_u128(0x12bbebe6_58d6_4636_95bb_3217ef867c1a);
const MAX_PERFORMANCE: u32 = 0;

pub fn powered_wifi() -> bool {
    let mut power = SYSTEM_POWER_STATUS::default();
    let Some(route) = latency::route_to(IP) else {
        return false;
    };
    let mut interface = MIB_IFROW {
        dwIndex: route.dwForwardIfIndex,
        ..Default::default()
    };
    // Both Windows APIs fill initialized output structures.
    unsafe {
        GetSystemPowerStatus(&mut power) != 0
            && power.ACLineStatus == 1
            && GetIfEntry(&mut interface) == 0
            && interface.dwType == 71
    }
}

fn backup_path() -> Option<PathBuf> {
    Some(
        PathBuf::from(env::var_os("LOCALAPPDATA")?)
            .join("NetDoctor")
            .join("wifi-restore"),
    )
}

pub fn can_restore() -> bool {
    backup_path().is_some_and(|path| path.is_file())
}

fn active_scheme() -> Option<GUID> {
    let mut scheme = null_mut();
    // PowerGetActiveScheme returns a locally allocated GUID, which LocalFree releases.
    unsafe {
        if PowerGetActiveScheme(null_mut(), &mut scheme) != 0 || scheme.is_null() {
            return None;
        }
        let value = *scheme;
        LocalFree(scheme.cast());
        Some(value)
    }
}

fn guid_number(scheme: GUID) -> u128 {
    (u128::from(scheme.data1) << 96)
        | (u128::from(scheme.data2) << 80)
        | (u128::from(scheme.data3) << 64)
        | u128::from(u64::from_be_bytes(scheme.data4))
}

fn read(scheme: &GUID) -> Option<u32> {
    let mut index = 0;
    // Read the AC value for this specific scheme and wireless setting.
    (unsafe { PowerReadACValueIndex(null_mut(), scheme, &WIRELESS, &SAVING, &mut index) } == 0)
        .then_some(index)
}

fn write(scheme: &GUID, index: u32) -> bool {
    // Reload only if the saved scheme is still active. Never select a different plan.
    unsafe {
        if PowerWriteACValueIndex(null_mut(), scheme, &WIRELESS, &SAVING, index) != 0 {
            return false;
        }
        if active_scheme().map(guid_number) == Some(guid_number(*scheme))
            && PowerSetActiveScheme(null_mut(), scheme) != 0
        {
            return false;
        }
    }
    read(scheme) == Some(index)
}

pub struct Change {
    path: PathBuf,
    lock: File,
    scheme: GUID,
    previous: u32,
}

impl Change {
    pub fn start() -> Option<Self> {
        if !powered_wifi() {
            return None;
        }
        let scheme = active_scheme()?;
        let previous = read(&scheme).filter(|value| (1..=3).contains(value))?;
        let path = backup_path()?;
        fs::create_dir_all(path.parent()?).ok()?;
        let mut lock = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&path)
            .ok()?;
        if writeln!(lock, "{:032x} {previous}", guid_number(scheme))
            .and_then(|()| lock.sync_all())
            .is_err()
        {
            drop(lock);
            let _ = fs::remove_file(path);
            return None;
        }
        let change = Self {
            path,
            lock,
            scheme,
            previous,
        };
        if !powered_wifi()
            || active_scheme().map(guid_number) != Some(guid_number(scheme))
            || read(&scheme) != Some(previous)
        {
            drop(change.lock);
            let _ = fs::remove_file(change.path);
            return None;
        }
        if !write(&scheme, MAX_PERFORMANCE) {
            change.undo();
            return None;
        }
        Some(change)
    }

    pub fn undo(self) -> bool {
        let restored = match read(&self.scheme) {
            Some(MAX_PERFORMANCE) => write(&self.scheme, self.previous),
            Some(value) => value == self.previous,
            None => false,
        };
        drop(self.lock);
        restored && fs::remove_file(self.path).is_ok()
    }
}

pub fn restore() -> bool {
    let attempt = || {
        let path = backup_path()?;
        let mut lock = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .ok()?;
        let mut record = String::new();
        (&mut lock).take(40).read_to_string(&mut record).ok()?;
        let (id, previous) = record.trim_end().split_once(' ')?;
        let scheme = GUID::from_u128(u128::from_str_radix(id, 16).ok()?);
        let previous = previous
            .parse::<u32>()
            .ok()
            .filter(|value| (1..=3).contains(value))?;
        (id.len() == 32).then_some(Change {
            path,
            lock,
            scheme,
            previous,
        })
    };
    attempt().is_some_and(Change::undo)
}
