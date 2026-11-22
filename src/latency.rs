use serde::Serialize;
use std::{mem::size_of, net::Ipv4Addr, ptr::null, thread, time::Duration};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    NetworkManagement::IpHelper::{
        GetBestRoute, ICMP_ECHO_REPLY, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho,
        MIB_IPFORWARDROW,
    },
};

pub fn route_to(destination: Ipv4Addr) -> Option<MIB_IPFORWARDROW> {
    let mut route = MIB_IPFORWARDROW::default();
    // Windows stores IPv4 addresses as network-order bytes.
    let status = unsafe { GetBestRoute(u32::from_ne_bytes(destination.octets()), 0, &mut route) };
    (status == 0).then_some(route)
}

pub fn gateway(destination: Ipv4Addr) -> Option<Ipv4Addr> {
    let next_hop = route_to(destination)?.dwForwardNextHop;
    (next_hop != 0).then(|| Ipv4Addr::from(next_hop.to_ne_bytes()))
}

pub struct Echo(HANDLE);

impl Echo {
    pub fn open() -> Option<Self> {
        // The API returns INVALID_HANDLE_VALUE on failure.
        let handle = unsafe { IcmpCreateFile() };
        (!handle.is_null() && handle != INVALID_HANDLE_VALUE).then(|| Self(handle))
    }

    pub fn ping(&self, address: Ipv4Addr) -> Option<u32> {
        #[repr(C)]
        struct Reply {
            header: ICMP_ECHO_REPLY,
            extra: [u8; 64],
        }
        let mut reply = Reply {
            header: ICMP_ECHO_REPLY::default(),
            extra: [0; 64],
        };
        let data = [0u8; 32];
        // The reply buffer is aligned and includes room for the echoed data and ICMP error bytes.
        let count = unsafe {
            IcmpSendEcho(
                self.0,
                u32::from_ne_bytes(address.octets()),
                data.as_ptr().cast(),
                data.len() as u16,
                null(),
                (&mut reply as *mut Reply).cast(),
                size_of::<Reply>() as u32,
                650,
            )
        };
        (count != 0 && reply.header.Status == 0).then_some(reply.header.RoundTripTime)
    }
}

impl Drop for Echo {
    fn drop(&mut self) {
        // Each successful open owns exactly one ICMP handle.
        unsafe { IcmpCloseHandle(self.0) };
    }
}

#[derive(Serialize)]
pub struct Stats {
    median: Option<f64>,
    loss: usize,
}

pub fn baseline(address: Ipv4Addr) -> Option<Stats> {
    const ATTEMPTS: usize = 5;
    let echo = Echo::open()?;
    let mut replies = Vec::with_capacity(ATTEMPTS);
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(125));
        }
        replies.extend(echo.ping(address));
    }
    Some(Stats {
        loss: (ATTEMPTS - replies.len()) * 100 / ATTEMPTS,
        median: median(&mut replies),
    })
}

pub fn median(replies: &mut [u32]) -> Option<f64> {
    if replies.is_empty() {
        return None;
    }
    replies.sort_unstable();
    let middle = replies.len() / 2;
    Some((f64::from(replies[(replies.len() - 1) / 2]) + f64::from(replies[middle])) / 2.0)
}
