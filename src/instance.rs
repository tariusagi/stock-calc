//! Single-instance handling and switching between the two apps.
//!
//! Each app holds a named mutex while it runs; a second launch finds the
//! running app's window, brings it to the front and exits.

use std::{thread, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    Portfolio,
    Calculator,
}

impl App {
    /// The OS window title; also used to find the running instance.
    pub const fn title(self) -> &'static str {
        match self {
            App::Portfolio => "Stock Portfolio",
            App::Calculator => "Stock Calculator",
        }
    }

    const fn exe(self) -> &'static str {
        match self {
            App::Portfolio => "portfolio.exe",
            App::Calculator => "stockcalc.exe",
        }
    }

    const fn mutex_name(self) -> &'static str {
        match self {
            App::Portfolio => r"Local\StockCalc.Portfolio.SingleInstance",
            App::Calculator => r"Local\StockCalc.Calculator.SingleInstance",
        }
    }
}

/// Claims the single-instance lock for `app` for the life of this process.
/// Returns `false` when another instance already holds it.
pub fn claim(app: App) -> bool {
    sys::claim(app.mutex_name())
}

/// Called by a second launch: brings the already running instance to the
/// front, waiting briefly in case its window is still being created.
pub fn hand_over(app: App) {
    for _ in 0..30 {
        if activate(app) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Brings the running instance of `app` to the front. Returns `false` if it
/// isn't running.
pub fn activate(app: App) -> bool {
    sys::activate(app.title(), app.exe())
}

/// Activates `app` if it's running, otherwise starts it from the folder of
/// the current executable.
pub fn open(app: App) -> Result<(), String> {
    if activate(app) {
        return Ok(());
    }
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(app.exe())))
        .filter(|p| p.exists())
        .ok_or_else(|| format!("{} was not found in the program folder", app.exe()))?;
    // Let the new process put its window in front of ours.
    sys::allow_foreground();
    std::process::Command::new(&exe)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not start {}: {e}", app.exe()))
}

#[cfg(windows)]
mod sys {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND};
    use windows_sys::Win32::System::Threading::{
        CreateMutexW, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        ASFW_ANY, AllowSetForegroundWindow, FindWindowExW, GetWindowThreadProcessId, IsIconic,
        SW_RESTORE, SetForegroundWindow, ShowWindow,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn claim(name: &str) -> bool {
        let name = wide(name);
        // SAFETY: plain Win32 calls with a valid, NUL-terminated name.
        unsafe {
            let handle = CreateMutexW(null(), 0, name.as_ptr());
            if handle.is_null() {
                return true; // can't tell; don't block the app from starting
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                return false;
            }
        }
        // The handle is intentionally left open until the process exits.
        true
    }

    pub fn activate(title: &str, exe: &str) -> bool {
        let Some(hwnd) = find_window(title, exe) else { return false };
        // SAFETY: `hwnd` is a window handle just returned by FindWindowExW.
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
        true
    }

    pub fn allow_foreground() {
        // SAFETY: no pointers involved.
        unsafe {
            AllowSetForegroundWindow(ASFW_ANY);
        }
    }

    /// Finds a top-level window with this title that belongs to `exe`, so an
    /// unrelated window (say, a browser tab) with the same title is ignored.
    fn find_window(title: &str, exe: &str) -> Option<HWND> {
        let title = wide(title);
        let mut hwnd: HWND = null_mut();
        loop {
            // SAFETY: valid NUL-terminated title; `hwnd` is null or a handle from the previous call.
            hwnd = unsafe { FindWindowExW(null_mut(), hwnd, null(), title.as_ptr()) };
            if hwnd.is_null() {
                return None;
            }
            if exe_name(hwnd).is_some_and(|n| n.eq_ignore_ascii_case(exe)) {
                return Some(hwnd);
            }
        }
    }

    fn exe_name(hwnd: HWND) -> Option<String> {
        let mut pid = 0u32;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: out-pointers reference live locals; the process handle is closed below.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 {
                return None;
            }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(process);
            if ok == 0 {
                return None;
            }
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_owned)
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn claim(_name: &str) -> bool {
        true
    }
    pub fn activate(_title: &str, _exe: &str) -> bool {
        false
    }
    pub fn allow_foreground() {}
}
