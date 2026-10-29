//! Small HTTP(S) download through Windows' own WinHTTP (proxy settings,
//! certificates and TLS come from Windows), used to bring the pictures of a
//! web page into a pasted note.

use std::ptr;

use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts, URL_COMPONENTS, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

use crate::win;

/// Closes a WinHTTP handle when dropped.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a handle WinHTTP gave us, closed once.
            unsafe {
                WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// True for an address on the public internet: not loopback, private
/// (RFC 1918 / unique local), link-local, unspecified, multicast or CGNAT.
fn is_public(address: &std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || (a == 100 && (64..128).contains(&b))
                || a == 0)
        }
        std::net::IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(&std::net::IpAddr::V4(v4));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xFE00) == 0xFC00
                || (first & 0xFFC0) == 0xFE80)
        }
    }
}

/// GET `url` (http or https); `None` on any error, a status other than 200,
/// or more than `max_bytes`. Waits at most a few seconds. Pasted content
/// decides the address, so only hosts on the public internet are contacted
/// (no loopback, LAN or link-local addresses) and redirects are not followed.
pub fn get(url: &str, max_bytes: usize) -> Option<Vec<u8>> {
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return None;
    }
    let wide = win::wide(url);
    let mut host = vec![0u16; 256];
    let mut path = vec![0u16; 4096];
    // SAFETY: all buffers outlive the calls; lengths are passed; every
    // handle is closed by `Handle`.
    unsafe {
        let mut parts: URL_COMPONENTS = std::mem::zeroed();
        parts.dwStructSize = std::mem::size_of::<URL_COMPONENTS>() as u32;
        parts.lpszHostName = host.as_mut_ptr();
        parts.dwHostNameLength = host.len() as u32;
        parts.lpszUrlPath = path.as_mut_ptr();
        parts.dwUrlPathLength = path.len() as u32;
        // The query string is part of the request too.
        let mut extra = vec![0u16; 4096];
        parts.lpszExtraInfo = extra.as_mut_ptr();
        parts.dwExtraInfoLength = extra.len() as u32;
        if WinHttpCrackUrl(wide.as_ptr(), 0, 0, &mut parts) == 0 {
            return None;
        }
        let secure = lower.starts_with("https://");
        let mut target: Vec<u16> = path[..parts.dwUrlPathLength as usize].to_vec();
        target.extend_from_slice(&extra[..parts.dwExtraInfoLength as usize]);
        target.push(0);
        let host_text = String::from_utf16_lossy(&host[..parts.dwHostNameLength as usize]);
        let addresses: Vec<std::net::SocketAddr> =
            std::net::ToSocketAddrs::to_socket_addrs(&(host_text.as_str(), parts.nPort))
                .ok()?
                .collect();
        if addresses.is_empty() || !addresses.iter().all(|address| is_public(&address.ip())) {
            return None;
        }
        let host_name: Vec<u16> = host[..parts.dwHostNameLength as usize]
            .iter()
            .copied()
            .chain([0])
            .collect();

        let agent = win::wide("Nodi");
        // 4 = WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY (Windows' proxy settings).
        let session = Handle(WinHttpOpen(agent.as_ptr(), 4, ptr::null(), ptr::null(), 0));
        if session.0.is_null() {
            return None;
        }
        WinHttpSetTimeouts(session.0, 4000, 4000, 4000, 8000);
        let connection = Handle(WinHttpConnect(
            session.0,
            host_name.as_ptr(),
            parts.nPort,
            0,
        ));
        if connection.0.is_null() {
            return None;
        }
        let verb = win::wide("GET");
        let request = Handle(WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            target.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            if secure { WINHTTP_FLAG_SECURE } else { 0 },
        ));
        if request.0.is_null() {
            return None;
        }
        // A redirect could point at an internal address: refuse them all.
        let never: u32 = 0; // WINHTTP_OPTION_REDIRECT_POLICY_NEVER
        WinHttpSetOption(
            request.0,
            88, // WINHTTP_OPTION_REDIRECT_POLICY
            &never as *const u32 as *const core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        );
        if WinHttpSendRequest(request.0, ptr::null(), 0, ptr::null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, ptr::null_mut()) == 0
        {
            return None;
        }
        let mut status = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            &mut status as *mut u32 as *mut core::ffi::c_void,
            &mut size,
            ptr::null_mut(),
        );
        if status != 200 {
            return None;
        }
        let mut body = Vec::new();
        loop {
            let mut available = 0u32;
            if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                return None;
            }
            if available == 0 {
                break;
            }
            if body.len() + available as usize > max_bytes {
                return None;
            }
            let start = body.len();
            body.resize(start + available as usize, 0);
            let mut read = 0u32;
            if WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr() as *mut core::ffi::c_void,
                available,
                &mut read,
            ) == 0
            {
                return None;
            }
            body.truncate(start + read as usize);
        }
        Some(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_are_fetched() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.10.12",
            "172.16.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:192.168.1.1",
        ] {
            let address: std::net::IpAddr = private.parse().unwrap();
            assert!(!is_public(&address), "{private}");
        }
        for public in ["8.8.8.8", "140.82.112.3", "2606:4700::1111"] {
            let address: std::net::IpAddr = public.parse().unwrap();
            assert!(is_public(&address), "{public}");
        }
        assert_eq!(get("http://127.0.0.1:9/x.png", 1024), None);
        assert_eq!(get("file:///C:/x.png", 1024), None);
    }
}
