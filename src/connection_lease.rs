//! Prevent two copies of this app from issuing overlapping F75 protocol requests.
#[cfg(windows)]
pub struct ConnectionLease(*mut std::ffi::c_void);
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(
        attributes: *const std::ffi::c_void,
        owner: i32,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn GetLastError() -> u32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}
#[cfg(windows)]
impl ConnectionLease {
    pub fn acquire() -> Result<Self, String> {
        let name = "Local\\AulaStudioF75Connection"
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // The handle remains alive for the connection lifetime. A named object is enough
        // to detect another live connection; thread-owned mutex locking is not needed.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if unsafe { GetLastError() } == 183 {
            unsafe { CloseHandle(handle) };
            return Err(
                "Another Aula Studio connection is open. Close that copy before connecting.".into(),
            );
        }
        Ok(Self(handle))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::ConnectionLease;
    #[test]
    fn second_connection_is_rejected_and_drop_releases_the_guard() {
        let first = ConnectionLease::acquire().unwrap();
        assert!(ConnectionLease::acquire().is_err());
        drop(first);
        assert!(ConnectionLease::acquire().is_ok());
    }
}
#[cfg(windows)]
impl Drop for ConnectionLease {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
#[cfg(not(windows))]
pub struct ConnectionLease;
#[cfg(not(windows))]
impl ConnectionLease {
    pub fn acquire() -> Result<Self, String> {
        Ok(Self)
    }
}
